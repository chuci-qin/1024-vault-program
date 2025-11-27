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
}
