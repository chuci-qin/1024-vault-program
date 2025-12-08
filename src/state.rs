//! Vault Program State Definitions
//!
//! Vault Program 职责: 纯用户资金托管 (用户的钱)
//! 
//! 架构原则:
//! - Vault Program = 用户资金托管 (入金/出金/保证金)
//! - Fund Program = 资金池管理 (保险基金/手续费/返佣等)
//!
//! 详见: onchain-program/vault_vs_fund.md

use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::pubkey::Pubkey;

/// VaultConfig 账户大小 (bytes)
/// 注意: Vec 和 Option 在 Borsh 中有额外的长度前缀
pub const VAULT_CONFIG_SIZE: usize = 8 + // discriminator
    32 + // admin
    32 + // usdc_mint
    32 + // vault_token_account
    4 + (32 * 10) + // authorized_callers (Vec<Pubkey>: 4字节长度 + 最多10个Pubkey)
    32 + // ledger_program
    1 + 32 + // fund_program (Option<Pubkey>: 1字节tag + 32字节)
    32 + // delegation_program
    8 + // total_deposits
    8 + // total_locked
    1 + // is_paused
    64; // 预留空间

/// UserAccount 账户大小 (bytes)
pub const USER_ACCOUNT_SIZE: usize = 8 + // discriminator
    32 + // wallet
    1 + // bump
    8 + // available_balance_e6
    8 + // locked_margin_e6
    8 + // unrealized_pnl_e6
    8 + // total_deposited_e6
    8 + // total_withdrawn_e6
    8 + // last_update_ts
    64; // reserved

/// Vault 全局配置
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone)]
pub struct VaultConfig {
    /// 账户类型标识符
    pub discriminator: u64,
    
    /// 管理员
    pub admin: Pubkey,
    
    /// USDC Mint 地址
    pub usdc_mint: Pubkey,
    
    /// Vault Token Account (存放所有用户的USDC)
    pub vault_token_account: Pubkey,
    
    /// 授权调用 CPI 的 Program 列表
    pub authorized_callers: Vec<Pubkey>,
    
    /// Ledger Program ID
    pub ledger_program: Pubkey,
    
    /// Fund Program ID (用于清算罚金转入等 CPI)
    pub fund_program: Option<Pubkey>,
    
    /// Delegation Program ID
    pub delegation_program: Pubkey,
    
    /// 总存款 (e6)
    pub total_deposits: u64,
    
    /// 总锁定保证金 (e6)
    pub total_locked: u64,
    
    /// 是否暂停
    pub is_paused: bool,
}

impl VaultConfig {
    pub const DISCRIMINATOR: u64 = 0x5641554C545F434F; // "VAULT_CO"
    
    /// 验证调用方是否授权
    pub fn is_authorized_caller(&self, caller: &Pubkey) -> bool {
        caller == &self.ledger_program 
            || self.fund_program.as_ref() == Some(caller)
            || self.authorized_callers.contains(caller)
    }
}

/// 用户账户 (PDA)
/// Seeds: ["user", wallet.key()]
/// 
/// 记录单个用户的保证金状态
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone)]
pub struct UserAccount {
    /// 账户类型标识符
    pub discriminator: u64,
    
    /// 用户钱包地址
    pub wallet: Pubkey,
    
    /// PDA bump
    pub bump: u8,
    
    /// 可用余额 (e6)
    pub available_balance_e6: i64,
    
    /// 锁定的保证金 (e6)
    pub locked_margin_e6: i64,
    
    /// 未实现盈亏 (e6) - 由 Position 更新
    pub unrealized_pnl_e6: i64,
    
    /// 累计存款 (e6)
    pub total_deposited_e6: i64,
    
    /// 累计提款 (e6)
    pub total_withdrawn_e6: i64,
    
    /// 最后更新时间戳
    pub last_update_ts: i64,
    
    /// 预留字段 (扩展用)
    pub reserved: [u8; 64],
}

impl UserAccount {
    pub const DISCRIMINATOR: u64 = 0x555345525F414343; // "USER_ACC"
    
    /// 计算权益 (Equity)
    pub fn equity(&self) -> i64 {
        self.available_balance_e6 + self.locked_margin_e6 + self.unrealized_pnl_e6
    }
}

// =============================================================================
// Prediction Market 专用账户 (独立 PDA，不修改现有结构)
// =============================================================================

/// PredictionMarketUserAccount discriminator
pub const PREDICTION_MARKET_USER_ACCOUNT_DISCRIMINATOR: u64 = 0x504D5F55534552; // "PM_USER"

/// PredictionMarketUserAccount PDA seed
pub const PREDICTION_MARKET_USER_SEED: &[u8] = b"prediction_market_user";

/// PredictionMarketUserAccount 账户大小 (bytes)
pub const PREDICTION_MARKET_USER_ACCOUNT_SIZE: usize = 8 + // discriminator
    32 + // wallet
    1 + // bump
    8 + // prediction_market_locked_e6
    8 + // prediction_market_pending_settlement_e6
    8 + // prediction_market_total_deposited_e6
    8 + // prediction_market_total_withdrawn_e6
    8 + // prediction_market_realized_pnl_e6
    8 + // last_update_ts
    64; // reserved

/// 预测市场用户账户 (PDA)
/// Seeds: ["prediction_market_user", wallet.key()]
/// 
/// 独立于 Perp 的 UserAccount，专门记录预测市场相关资金状态
/// 
/// 设计原则：
/// - 不修改现有的 UserAccount 结构
/// - 预测市场用户使用独立的 PDA 存储资金状态
/// - 资金从 UserAccount.available_balance 转入/转出
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone)]
pub struct PredictionMarketUserAccount {
    /// 账户类型标识符
    pub discriminator: u64,
    
    /// 用户钱包地址
    pub wallet: Pubkey,
    
    /// PDA bump
    pub bump: u8,
    
    /// 预测市场锁定资金 (e6)
    /// 用户购买 YES/NO Token 时从 UserAccount.available_balance 扣除并锁定到此处
    pub prediction_market_locked_e6: i64,
    
    /// 预测市场待结算收益 (e6)
    /// 市场结算后用户应得的 USDC，等待用户主动领取
    pub prediction_market_pending_settlement_e6: i64,
    
    /// 预测市场累计存入 (e6)
    /// 从 UserAccount 转入的总金额
    pub prediction_market_total_deposited_e6: i64,
    
    /// 预测市场累计提取 (e6)
    /// 转回 UserAccount 的总金额
    pub prediction_market_total_withdrawn_e6: i64,
    
    /// 预测市场已实现盈亏 (e6)
    pub prediction_market_realized_pnl_e6: i64,
    
    /// 最后更新时间戳
    pub last_update_ts: i64,
    
    /// 预留字段 (扩展用)
    pub reserved: [u8; 64],
}

impl PredictionMarketUserAccount {
    pub const DISCRIMINATOR: u64 = PREDICTION_MARKET_USER_ACCOUNT_DISCRIMINATOR;
    
    /// PDA seeds
    pub fn seeds(wallet: &Pubkey) -> Vec<Vec<u8>> {
        vec![
            PREDICTION_MARKET_USER_SEED.to_vec(),
            wallet.to_bytes().to_vec(),
        ]
    }
    
    /// 创建新的预测市场用户账户
    pub fn new(wallet: Pubkey, bump: u8, created_at: i64) -> Self {
        Self {
            discriminator: Self::DISCRIMINATOR,
            wallet,
            bump,
            prediction_market_locked_e6: 0,
            prediction_market_pending_settlement_e6: 0,
            prediction_market_total_deposited_e6: 0,
            prediction_market_total_withdrawn_e6: 0,
            prediction_market_realized_pnl_e6: 0,
            last_update_ts: created_at,
            reserved: [0u8; 64],
        }
    }
    
    /// 计算预测市场权益
    pub fn prediction_market_equity(&self) -> i64 {
        self.prediction_market_locked_e6 + self.prediction_market_pending_settlement_e6
    }
    
    /// 锁定资金用于预测市场
    /// 增加 prediction_market_locked_e6
    pub fn prediction_market_lock(&mut self, amount: i64, current_ts: i64) {
        self.prediction_market_locked_e6 += amount;
        self.prediction_market_total_deposited_e6 += amount;
        self.last_update_ts = current_ts;
    }
    
    /// 释放预测市场锁定资金
    pub fn prediction_market_unlock(&mut self, amount: i64, current_ts: i64) -> Result<(), &'static str> {
        if self.prediction_market_locked_e6 < amount {
            return Err("Insufficient prediction market locked amount");
        }
        self.prediction_market_locked_e6 -= amount;
        self.prediction_market_total_withdrawn_e6 += amount;
        self.last_update_ts = current_ts;
        Ok(())
    }
    
    /// 预测市场结算
    /// 释放锁定并记录结算收益
    pub fn prediction_market_settle(
        &mut self, 
        locked_to_release: i64, 
        settlement_amount: i64,
        current_ts: i64,
    ) -> Result<(), &'static str> {
        if self.prediction_market_locked_e6 < locked_to_release {
            return Err("Insufficient prediction market locked amount");
        }
        self.prediction_market_locked_e6 -= locked_to_release;
        self.prediction_market_pending_settlement_e6 += settlement_amount;
        
        // 计算盈亏
        let pnl = settlement_amount - locked_to_release;
        self.prediction_market_realized_pnl_e6 += pnl;
        
        self.last_update_ts = current_ts;
        Ok(())
    }
    
    /// 领取预测市场结算收益
    /// 清空 prediction_market_pending_settlement_e6
    pub fn prediction_market_claim_settlement(&mut self, current_ts: i64) -> i64 {
        let amount = self.prediction_market_pending_settlement_e6;
        self.prediction_market_pending_settlement_e6 = 0;
        self.prediction_market_total_withdrawn_e6 += amount;
        self.last_update_ts = current_ts;
        amount
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_user_account_equity() {
        let account = UserAccount {
            discriminator: UserAccount::DISCRIMINATOR,
            wallet: Pubkey::new_unique(),
            bump: 255,
            available_balance_e6: 1000_000_000,
            locked_margin_e6: 500_000_000,
            unrealized_pnl_e6: 200_000_000,
            total_deposited_e6: 1000_000_000,
            total_withdrawn_e6: 0,
            last_update_ts: 0,
            reserved: [0; 64],
        };
        
        assert_eq!(account.equity(), 1700_000_000);
    }

    #[test]
    fn test_vault_config_authorized_caller() {
        let ledger = Pubkey::new_unique();
        let fund = Pubkey::new_unique();
        let other = Pubkey::new_unique();
        let authorized = Pubkey::new_unique();
        
        let config = VaultConfig {
            discriminator: VaultConfig::DISCRIMINATOR,
            admin: Pubkey::new_unique(),
            usdc_mint: Pubkey::new_unique(),
            vault_token_account: Pubkey::new_unique(),
            authorized_callers: vec![authorized],
            ledger_program: ledger,
            fund_program: Some(fund),
            delegation_program: Pubkey::new_unique(),
            total_deposits: 0,
            total_locked: 0,
            is_paused: false,
        };
        
        assert!(config.is_authorized_caller(&ledger));
        assert!(config.is_authorized_caller(&fund));
        assert!(config.is_authorized_caller(&authorized));
        assert!(!config.is_authorized_caller(&other));
    }

    // === Prediction Market User Account Tests ===

    #[test]
    fn test_prediction_market_user_account_size() {
        assert!(PREDICTION_MARKET_USER_ACCOUNT_SIZE > 0);
        println!("PredictionMarketUserAccount SIZE: {}", PREDICTION_MARKET_USER_ACCOUNT_SIZE);
    }

    #[test]
    fn test_prediction_market_user_account_creation() {
        let wallet = Pubkey::new_unique();
        let account = PredictionMarketUserAccount::new(wallet, 255, 1000);
        
        assert_eq!(account.wallet, wallet);
        assert_eq!(account.prediction_market_locked_e6, 0);
        assert_eq!(account.prediction_market_pending_settlement_e6, 0);
        assert_eq!(account.prediction_market_equity(), 0);
    }

    #[test]
    fn test_prediction_market_lock_unlock() {
        let wallet = Pubkey::new_unique();
        let mut account = PredictionMarketUserAccount::new(wallet, 255, 1000);
        
        // Lock funds
        account.prediction_market_lock(100_000_000, 1001);
        assert_eq!(account.prediction_market_locked_e6, 100_000_000);
        assert_eq!(account.prediction_market_total_deposited_e6, 100_000_000);
        
        // Unlock funds
        account.prediction_market_unlock(50_000_000, 1002).unwrap();
        assert_eq!(account.prediction_market_locked_e6, 50_000_000);
        assert_eq!(account.prediction_market_total_withdrawn_e6, 50_000_000);
        
        // Try to unlock more than available - should fail
        assert!(account.prediction_market_unlock(100_000_000, 1003).is_err());
    }

    #[test]
    fn test_prediction_market_settle() {
        let wallet = Pubkey::new_unique();
        let mut account = PredictionMarketUserAccount::new(wallet, 255, 1000);
        
        // Lock 100 USDC
        account.prediction_market_lock(100_000_000, 1001);
        
        // Settle with profit (YES wins, get 100 USDC back)
        account.prediction_market_settle(100_000_000, 100_000_000, 1002).unwrap();
        assert_eq!(account.prediction_market_locked_e6, 0);
        assert_eq!(account.prediction_market_pending_settlement_e6, 100_000_000);
        assert_eq!(account.prediction_market_realized_pnl_e6, 0); // Break even
        
        // Claim
        let claimed = account.prediction_market_claim_settlement(1003);
        assert_eq!(claimed, 100_000_000);
        assert_eq!(account.prediction_market_pending_settlement_e6, 0);
    }

    #[test]
    fn test_prediction_market_settle_with_profit() {
        let wallet = Pubkey::new_unique();
        let mut account = PredictionMarketUserAccount::new(wallet, 255, 1000);
        
        // Lock 50 USDC (bought YES at $0.50)
        account.prediction_market_lock(50_000_000, 1001);
        
        // Settle with profit (YES wins, get 100 USDC back - 100 tokens * $1)
        account.prediction_market_settle(50_000_000, 100_000_000, 1002).unwrap();
        assert_eq!(account.prediction_market_realized_pnl_e6, 50_000_000); // +$50 profit
    }
}
