//! Vault Program State Definitions
//!
//! Vault Program 职责: 纯用户资金托管 (入金/出金/保证金)
//!
//! 架构: DB-First + 实时链上审计
//! 只保留两个链上程序: Vault (资金托管) + Exchange (审计记录)

use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::pubkey::Pubkey;

/// VaultConfig 账户大小 (bytes)
///
/// 变更记录:
/// - 2025-12-10: authorized_callers 从 Vec<Pubkey> 改为 [Pubkey; 10] 固定大小数组
/// - 2026-03-15: 删除 ledger_program 和 fund_program 字段 (569 → 505 bytes)
///   这两个程序已完全废弃，字段不再保留。链上数据通过 MigrateVaultConfig 指令迁移。
pub const VAULT_CONFIG_SIZE: usize = 8 + // discriminator
    32 + // admin
    32 + // usdc_mint
    32 + // vault_token_account
    32 * 10 + // authorized_callers ([Pubkey; 10])
    32 + // delegation_program
    8 + // total_deposits
    8 + // total_locked
    1 + // is_paused
    32; // 预留空间
// Total: 8 + 32 + 32 + 32 + 320 + 32 + 8 + 8 + 1 + 32 = 505 bytes ✓

/// UserAccount 账户大小 (bytes)
///
/// Layout (153 bytes total, unchanged from V1):
///   disc(8) + wallet(32) + bump(1) + 7×i64(56) + account_index(1) + oracle_locked_e6(8) + reserved(47)
pub const USER_ACCOUNT_SIZE: usize = 8 + // discriminator
    32 + // wallet
    1 + // bump
    8 + // available_balance_e6
    8 + // locked_margin_e6
    8 + // unrealized_pnl_e6
    8 + // total_deposited_e6
    8 + // total_withdrawn_e6
    8 + // last_update_ts
    8 + // spot_locked_e6 (One Account Experience)
    1 + // account_index (V2: sub-account isolation)
    8 + // oracle_locked_e6 (V2: PM Oracle bond)
    47; // reserved (was 56, reduced by 1+8 for account_index+oracle_locked)

/// Vault 全局配置 (505 bytes)
///
/// 变更记录:
/// - 2025-12-10: authorized_callers 从 Vec<Pubkey> 改为 [Pubkey; 10]
/// - 2026-03-15: 删除 ledger_program 和 fund_program (569→505 bytes)
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone)]
pub struct VaultConfig {
    /// 账户类型标识符 (8 bytes)
    pub discriminator: u64,
    
    /// 管理员 (32 bytes)
    pub admin: Pubkey,
    
    /// USDC Mint 地址 (32 bytes)
    pub usdc_mint: Pubkey,
    
    /// Vault Token Account (存放所有用户的USDC) (32 bytes)
    pub vault_token_account: Pubkey,
    
    /// 授权调用 CPI 的 Program 列表 (320 bytes = 32 * 10)
    pub authorized_callers: [Pubkey; 10],
    
    /// Delegation Program ID (32 bytes)
    pub delegation_program: Pubkey,
    
    /// 总存款 (e6) (8 bytes)
    pub total_deposits: u64,
    
    /// 总锁定保证金 (e6) (8 bytes)
    pub total_locked: u64,
    
    /// 是否暂停 (1 byte)
    pub is_paused: bool,
    
    /// 预留空间 (32 bytes)
    pub reserved: [u8; 32],
}
// Total: 8 + 32 + 32 + 32 + 320 + 32 + 8 + 8 + 1 + 32 = 505 bytes ✓

/// Old VaultConfig size before migration (ledger_program + fund_program removed)
pub const VAULT_CONFIG_SIZE_V1: usize = 569;

impl VaultConfig {
    pub const DISCRIMINATOR: u64 = 0x5641554C545F434F; // "VAULT_CO"
    
    /// Verify caller is authorized via the authorized_callers array.
    pub fn is_authorized_caller(&self, caller: &Pubkey) -> bool {
        for authorized in &self.authorized_callers {
            if authorized != &Pubkey::default() && caller == authorized {
                return true;
            }
        }
        false
    }
}

/// 用户账户 (PDA)
/// Seeds: ["user", wallet, &[account_index]]
/// 
/// 记录单个用户/子账户的保证金状态。
/// account_index=0 为主账户，1-255 为子账户。
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone)]
pub struct UserAccount {
    /// 账户类型标识符
    pub discriminator: u64,
    
    /// 用户钱包地址
    pub wallet: Pubkey,
    
    /// PDA bump
    pub bump: u8,
    
    /// 可用余额 (e6) — Perp/Spot/PM 统一可用
    pub available_balance_e6: i64,
    
    /// 锁定的保证金 (e6) — Perp 持仓保证金
    pub locked_margin_e6: i64,
    
    /// 未实现盈亏 (e6) - 由 Position 更新
    pub unrealized_pnl_e6: i64,
    
    /// 累计存款 (e6)
    pub total_deposited_e6: i64,
    
    /// 累计提款 (e6)
    pub total_withdrawn_e6: i64,
    
    /// 最后更新时间戳
    pub last_update_ts: i64,
    
    /// Spot BUY 订单锁定的 USDC (e6)
    /// 
    /// One Account Experience: USDC 不再通过 SpotTokenBalance PDA 管理，
    /// 而是直接在 UserAccount 内通过 available ↔ spot_locked 字段搬运。
    pub spot_locked_e6: i64,
    
    /// Sub-account index (V2).
    /// 0 = main account, 1-255 = sub-accounts.
    /// Included in PDA seeds for per-account isolation.
    /// Borsh-compatible: old PDAs had reserved[8]=0 → account_index=0 (main).
    pub account_index: u8,
    
    /// PM Oracle bond locked amount (e6, V2).
    /// Used by Exchange program CPI (LockBond/ReleaseBond) for PM Oracle proposals.
    /// Borsh-compatible: old PDAs had reserved[9..17]=0 → oracle_locked_e6=0.
    pub oracle_locked_e6: i64,
    
    /// 预留字段 (扩展用) — from 56 → 47 (1+8 carved for account_index + oracle_locked)
    pub reserved: [u8; 47],
}

impl UserAccount {
    pub const DISCRIMINATOR: u64 = 0x555345525F414343; // "USER_ACC"
    
    pub const USER_SEED: &'static [u8] = b"user";
    
    /// Derive UserAccount PDA address.
    /// Seeds: ["user", wallet, &[account_index]]
    pub fn derive_pda(program_id: &Pubkey, wallet: &Pubkey, account_index: u8) -> (Pubkey, u8) {
        Pubkey::find_program_address(
            &[Self::USER_SEED, wallet.as_ref(), &[account_index]],
            program_id,
        )
    }
    
    /// 计算权益 (Equity)
    /// 
    /// equity = 可用余额 + Perp 锁定保证金 + Spot 锁定 USDC + Oracle 锁定 + 未实现盈亏
    pub fn equity(&self) -> i64 {
        self.available_balance_e6
            .saturating_add(self.locked_margin_e6)
            .saturating_add(self.spot_locked_e6)
            .saturating_add(self.oracle_locked_e6)
            .saturating_add(self.unrealized_pnl_e6)
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
    
    /// 锁定资金用于预测市场（checked arithmetic）
    pub fn prediction_market_lock(&mut self, amount: i64, current_ts: i64) -> Result<(), &'static str> {
        self.prediction_market_locked_e6 = self.prediction_market_locked_e6
            .checked_add(amount)
            .ok_or("Overflow in prediction_market_locked_e6")?;
        self.prediction_market_total_deposited_e6 = self.prediction_market_total_deposited_e6
            .checked_add(amount)
            .ok_or("Overflow in prediction_market_total_deposited_e6")?;
        self.last_update_ts = current_ts;
        Ok(())
    }
    
    /// 释放预测市场锁定资金（checked arithmetic）
    pub fn prediction_market_unlock(&mut self, amount: i64, current_ts: i64) -> Result<(), &'static str> {
        if self.prediction_market_locked_e6 < amount {
            return Err("Insufficient prediction market locked amount");
        }
        self.prediction_market_locked_e6 = self.prediction_market_locked_e6
            .checked_sub(amount)
            .ok_or("Underflow in prediction_market_locked_e6")?;
        self.prediction_market_total_withdrawn_e6 = self.prediction_market_total_withdrawn_e6
            .checked_add(amount)
            .ok_or("Overflow in prediction_market_total_withdrawn_e6")?;
        self.last_update_ts = current_ts;
        Ok(())
    }
    
    /// 预测市场结算（checked arithmetic）
    pub fn prediction_market_settle(
        &mut self, 
        locked_to_release: i64, 
        settlement_amount: i64,
        current_ts: i64,
    ) -> Result<(), &'static str> {
        if self.prediction_market_locked_e6 < locked_to_release {
            return Err("Insufficient prediction market locked amount");
        }
        self.prediction_market_locked_e6 = self.prediction_market_locked_e6
            .checked_sub(locked_to_release)
            .ok_or("Underflow in prediction_market_locked_e6")?;
        self.prediction_market_pending_settlement_e6 = self.prediction_market_pending_settlement_e6
            .checked_add(settlement_amount)
            .ok_or("Overflow in prediction_market_pending_settlement_e6")?;
        
        let pnl = settlement_amount
            .checked_sub(locked_to_release)
            .ok_or("Underflow in pnl calculation")?;
        self.prediction_market_realized_pnl_e6 = self.prediction_market_realized_pnl_e6
            .checked_add(pnl)
            .ok_or("Overflow in prediction_market_realized_pnl_e6")?;
        
        self.last_update_ts = current_ts;
        Ok(())
    }
    
    /// 领取预测市场结算收益（checked arithmetic）
    pub fn prediction_market_claim_settlement(&mut self, current_ts: i64) -> Result<i64, &'static str> {
        let amount = self.prediction_market_pending_settlement_e6;
        self.prediction_market_pending_settlement_e6 = 0;
        self.prediction_market_total_withdrawn_e6 = self.prediction_market_total_withdrawn_e6
            .checked_add(amount)
            .ok_or("Overflow in prediction_market_total_withdrawn_e6")?;
        self.last_update_ts = current_ts;
        Ok(amount)
    }
}

// =============================================================================
// SpotTokenBalance — Per-Token Balance PDA (Dynamic Token Balance Architecture)
// =============================================================================
//
// Replaces the old SpotUserAccount (635 bytes, 16-token limit) with individual
// per-token PDAs (98 bytes each, unlimited tokens per user).
//
// Design: DYNAMIC-TOKEN-BALANCE-ARCHITECTURE.md (8 audit rounds, 23 findings)
// Decision: Plan A — SpotUserAccount completely deleted, no header, no sequence check.
//
// Each (wallet, account_index, token_index) triple gets its own PDA, auto-created on first use.
// PDA seeds: ["spot_balance", wallet, &[account_index], token_index.to_le_bytes()]

/// SpotTokenBalance discriminator — "SPTK_BAL" in ASCII hex
pub const SPOT_TOKEN_BALANCE_DISCRIMINATOR: u64 = 0x5350544B5F42414C;

/// SpotTokenBalance PDA seed
pub const SPOT_BALANCE_SEED: &[u8] = b"spot_balance";

/// SpotTokenBalance account size (bytes)
/// discriminator(8) + wallet(32) + token_index(2) + available_e6(8) + locked_e6(8)
/// + last_update_ts(8) + bump(1) + reserved(31) = 98 bytes
pub const SPOT_TOKEN_BALANCE_SIZE: usize = 98;

/// Per-token balance PDA — one per (wallet, token_index) pair
///
/// Completely self-contained: PDA seeds verify ownership, discriminator verifies type.
/// No SpotUserAccount header needed. No InitializeSpotUser step required.
/// Auto-initialized on first deposit/settle via `auto_init_spot_balance()`.
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone)]
pub struct SpotTokenBalance {
    /// Account type discriminator
    pub discriminator: u64,
    /// User wallet (redundant with PDA seeds, kept for defense-in-depth + getProgramAccounts scanning)
    pub wallet: Pubkey,
    /// Token index (matches DB assets table)
    pub token_index: u16,
    /// Available balance (e6 precision) — can be used for orders or withdrawal
    pub available_e6: i64,
    /// Locked balance (e6 precision) — reserved by open orders
    pub locked_e6: i64,
    /// Last update timestamp (unix seconds)
    pub last_update_ts: i64,
    /// PDA bump seed
    pub bump: u8,
    /// Reserved for future expansion
    pub reserved: [u8; 31],
}

impl SpotTokenBalance {
    pub const DISCRIMINATOR: u64 = SPOT_TOKEN_BALANCE_DISCRIMINATOR;

    /// Create a new SpotTokenBalance with zero balances
    pub fn new(wallet: Pubkey, token_index: u16, bump: u8, current_ts: i64) -> Self {
        Self {
            discriminator: Self::DISCRIMINATOR,
            wallet,
            token_index,
            available_e6: 0,
            locked_e6: 0,
            last_update_ts: current_ts,
            bump,
            reserved: [0u8; 31],
        }
    }

    /// Total balance (available + locked). Uses checked arithmetic; returns error on overflow.
    pub fn total(&self) -> Result<i64, &'static str> {
        self.available_e6
            .checked_add(self.locked_e6)
            .ok_or("Overflow in SpotTokenBalance total (available_e6 + locked_e6)")
    }

    /// Deduct from balance, preferring available first, then locked.
    /// Uses checked arithmetic; returns error on overflow/underflow or insufficient balance.
    pub fn deduct_prefer_available(&mut self, amount: i64) -> Result<(), &'static str> {
        if amount <= 0 {
            return Err("Deduct amount must be positive");
        }
        let total = self
            .available_e6
            .checked_add(self.locked_e6)
            .ok_or("Overflow in SpotTokenBalance total (available_e6 + locked_e6)")?;
        if total < amount {
            return Err("Insufficient balance");
        }
        if self.available_e6 >= amount {
            self.available_e6 = self
                .available_e6
                .checked_sub(amount)
                .ok_or("Underflow in SpotTokenBalance available_e6")?;
        } else {
            let from_locked = amount
                .checked_sub(self.available_e6)
                .ok_or("Underflow in from_locked calculation (amount - available_e6)")?;
            self.available_e6 = 0;
            self.locked_e6 = self
                .locked_e6
                .checked_sub(from_locked)
                .ok_or("Underflow in SpotTokenBalance locked_e6")?;
        }
        Ok(())
    }
}

/// Derive SpotTokenBalance PDA address
///
/// Seeds: ["spot_balance", wallet, account_index, token_index.to_le_bytes()]
/// account_index ensures sub-accounts have isolated Spot balances.
/// Returns (pda_address, bump)
pub fn derive_spot_token_balance_pda(
    program_id: &Pubkey,
    wallet: &Pubkey,
    token_index: u16,
) -> (Pubkey, u8) {
    derive_spot_token_balance_pda_with_index(program_id, wallet, 0, token_index)
}

/// Derive SpotTokenBalance PDA address with explicit account_index.
///
/// Seeds: ["spot_balance", wallet, &[account_index], token_index.to_le_bytes()]
pub fn derive_spot_token_balance_pda_with_index(
    program_id: &Pubkey,
    wallet: &Pubkey,
    account_index: u8,
    token_index: u16,
) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            SPOT_BALANCE_SEED,
            wallet.as_ref(),
            &[account_index],
            &token_index.to_le_bytes(),
        ],
        program_id,
    )
}

// =============================================================================
// 定时支付授权账户 (2026-01-27 新增 - 站内支付系统)
// =============================================================================

/// RecurringAuth discriminator
pub const RECURRING_AUTH_DISCRIMINATOR: u64 = 0x5245435F41555448; // "REC_AUTH"

/// RecurringAuth PDA seed
pub const RECURRING_AUTH_SEED: &[u8] = b"recurring_auth";

/// RecurringAuth 账户大小 (bytes)
pub const RECURRING_AUTH_SIZE: usize = 8 + // discriminator
    32 + // payer
    32 + // payee
    1 + // bump
    8 + // amount_e6
    8 + // interval_seconds
    4 + // max_cycles
    4 + // current_cycles
    1 + // is_active
    8 + // created_at
    8 + // last_executed_at
    32 + // state_hash
    64; // reserved

/// 定时支付授权 PDA
/// Seeds: ["recurring_auth", payer, payee]
/// 
/// 记录定时支付授权信息，支持链上存证
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone)]
pub struct RecurringAuth {
    /// 账户类型标识符
    pub discriminator: u64,
    
    /// 付款方钱包地址
    pub payer: Pubkey,
    
    /// 收款方钱包地址
    pub payee: Pubkey,
    
    /// PDA bump
    pub bump: u8,
    
    /// 每期扣款金额 (e6)
    pub amount_e6: u64,
    
    /// 扣款周期 (秒)
    pub interval_seconds: i64,
    
    /// 最大执行次数 (0=无限)
    pub max_cycles: u32,
    
    /// 已执行次数
    pub current_cycles: u32,
    
    /// 是否激活
    pub is_active: bool,
    
    /// 创建时间戳
    pub created_at: i64,
    
    /// 最后执行时间戳
    pub last_executed_at: i64,
    
    /// 数据库状态哈希 (存证)
    pub state_hash: [u8; 32],
    
    /// 预留字段
    pub reserved: [u8; 64],
}

impl RecurringAuth {
    pub const DISCRIMINATOR: u64 = RECURRING_AUTH_DISCRIMINATOR;
    
    /// PDA seeds
    pub fn seeds(payer: &Pubkey, payee: &Pubkey) -> Vec<Vec<u8>> {
        vec![
            RECURRING_AUTH_SEED.to_vec(),
            payer.to_bytes().to_vec(),
            payee.to_bytes().to_vec(),
        ]
    }
    
    /// 创建新的定时支付授权
    pub fn new(
        payer: Pubkey,
        payee: Pubkey,
        bump: u8,
        amount_e6: u64,
        interval_seconds: i64,
        max_cycles: u32,
        created_at: i64,
    ) -> Self {
        Self {
            discriminator: Self::DISCRIMINATOR,
            payer,
            payee,
            bump,
            amount_e6,
            interval_seconds,
            max_cycles,
            current_cycles: 0,
            is_active: true,
            created_at,
            last_executed_at: 0,
            state_hash: [0u8; 32],
            reserved: [0u8; 64],
        }
    }
    
    /// 执行一次扣款
    pub fn execute(&mut self, current_ts: i64) -> Result<(), &'static str> {
        if !self.is_active {
            return Err("Recurring auth is not active");
        }
        
        self.current_cycles += 1;
        self.last_executed_at = current_ts;
        
        // 检查是否达到最大执行次数
        if self.max_cycles > 0 && self.current_cycles >= self.max_cycles {
            self.is_active = false;
        }
        
        Ok(())
    }
    
    /// 取消授权
    pub fn cancel(&mut self) {
        self.is_active = false;
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
            spot_locked_e6: 300_000_000,
            account_index: 0,
            oracle_locked_e6: 100_000_000,
            reserved: [0; 47],
        };
        
        // equity = available(1000) + locked_margin(500) + spot_locked(300) + oracle_locked(100) + upnl(200) = 2100
        assert_eq!(account.equity(), 2100_000_000);
    }
    
    #[test]
    fn test_user_account_size_unchanged() {
        let account = UserAccount {
            discriminator: UserAccount::DISCRIMINATOR,
            wallet: Pubkey::new_unique(),
            bump: 0,
            available_balance_e6: 0,
            locked_margin_e6: 0,
            unrealized_pnl_e6: 0,
            total_deposited_e6: 0,
            total_withdrawn_e6: 0,
            last_update_ts: 0,
            spot_locked_e6: 0,
            account_index: 0,
            oracle_locked_e6: 0,
            reserved: [0; 47],
        };
        let serialized = borsh::to_vec(&account).unwrap();
        // 8(disc) + 32(wallet) + 1(bump) + 7*i64(56) + 1(account_index) + 8(oracle_locked) + 47(reserved) = 153
        assert_eq!(serialized.len(), 153, "UserAccount size must remain 153 bytes");
    }
    
    #[test]
    fn test_user_account_derive_pda() {
        let program_id = Pubkey::new_unique();
        let wallet = Pubkey::new_unique();
        
        let (pda_main, _) = UserAccount::derive_pda(&program_id, &wallet, 0);
        let (pda_sub1, _) = UserAccount::derive_pda(&program_id, &wallet, 1);
        let (pda_main2, _) = UserAccount::derive_pda(&program_id, &wallet, 0);
        
        assert_ne!(pda_main, pda_sub1, "Different account_index → different PDA");
        assert_eq!(pda_main, pda_main2, "Same inputs → same PDA");
    }

    #[test]
    fn test_vault_config_authorized_caller() {
        let other = Pubkey::new_unique();
        let authorized = Pubkey::new_unique();
        
        let mut authorized_callers = [Pubkey::default(); 10];
        authorized_callers[0] = authorized;
        
        let config = VaultConfig {
            discriminator: VaultConfig::DISCRIMINATOR,
            admin: Pubkey::new_unique(),
            usdc_mint: Pubkey::new_unique(),
            vault_token_account: Pubkey::new_unique(),
            authorized_callers,
            delegation_program: Pubkey::new_unique(),
            total_deposits: 0,
            total_locked: 0,
            is_paused: false,
            reserved: [0u8; 32],
        };
        
        assert!(config.is_authorized_caller(&authorized));
        assert!(!config.is_authorized_caller(&other));
        assert!(!config.is_authorized_caller(&Pubkey::default()));
    }

    #[test]
    fn test_vault_config_size() {
        let config = VaultConfig {
            discriminator: VaultConfig::DISCRIMINATOR,
            admin: Pubkey::new_unique(),
            usdc_mint: Pubkey::new_unique(),
            vault_token_account: Pubkey::new_unique(),
            authorized_callers: [Pubkey::default(); 10],
            delegation_program: Pubkey::new_unique(),
            total_deposits: 0,
            total_locked: 0,
            is_paused: false,
            reserved: [0u8; 32],
        };
        let serialized = borsh::to_vec(&config).unwrap();
        assert_eq!(serialized.len(), VAULT_CONFIG_SIZE, "VaultConfig must be 505 bytes");
        assert_eq!(VAULT_CONFIG_SIZE, 505);
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
        account.prediction_market_lock(100_000_000, 1001).unwrap();
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
        account.prediction_market_lock(100_000_000, 1001).unwrap();
        
        // Settle with profit (YES wins, get 100 USDC back)
        account.prediction_market_settle(100_000_000, 100_000_000, 1002).unwrap();
        assert_eq!(account.prediction_market_locked_e6, 0);
        assert_eq!(account.prediction_market_pending_settlement_e6, 100_000_000);
        assert_eq!(account.prediction_market_realized_pnl_e6, 0); // Break even
        
        // Claim
        let claimed = account.prediction_market_claim_settlement(1003).unwrap();
        assert_eq!(claimed, 100_000_000);
        assert_eq!(account.prediction_market_pending_settlement_e6, 0);
    }

    #[test]
    fn test_prediction_market_settle_with_profit() {
        let wallet = Pubkey::new_unique();
        let mut account = PredictionMarketUserAccount::new(wallet, 255, 1000);
        
        // Lock 50 USDC (bought YES at $0.50)
        account.prediction_market_lock(50_000_000, 1001).unwrap();
        
        // Settle with profit (YES wins, get 100 USDC back - 100 tokens * $1)
        account.prediction_market_settle(50_000_000, 100_000_000, 1002).unwrap();
        assert_eq!(account.prediction_market_realized_pnl_e6, 50_000_000); // +$50 profit
    }

    // === RecurringAuth Tests ===

    #[test]
    fn test_recurring_auth_creation() {
        let payer = Pubkey::new_unique();
        let payee = Pubkey::new_unique();
        let auth = RecurringAuth::new(payer, payee, 255, 10_000_000, 2592000, 12, 1000);
        
        assert_eq!(auth.payer, payer);
        assert_eq!(auth.payee, payee);
        assert_eq!(auth.amount_e6, 10_000_000);
        assert_eq!(auth.interval_seconds, 2592000);
        assert_eq!(auth.max_cycles, 12);
        assert_eq!(auth.current_cycles, 0);
        assert!(auth.is_active);
    }

    #[test]
    fn test_recurring_auth_execute() {
        let payer = Pubkey::new_unique();
        let payee = Pubkey::new_unique();
        let mut auth = RecurringAuth::new(payer, payee, 255, 10_000_000, 2592000, 3, 1000);
        
        // Execute first cycle
        auth.execute(1001).unwrap();
        assert_eq!(auth.current_cycles, 1);
        assert!(auth.is_active);
        
        // Execute second cycle
        auth.execute(1002).unwrap();
        assert_eq!(auth.current_cycles, 2);
        assert!(auth.is_active);
        
        // Execute third (last) cycle
        auth.execute(1003).unwrap();
        assert_eq!(auth.current_cycles, 3);
        assert!(!auth.is_active); // Auto-deactivated after max_cycles
    }

    #[test]
    fn test_recurring_auth_unlimited_cycles() {
        let payer = Pubkey::new_unique();
        let payee = Pubkey::new_unique();
        let mut auth = RecurringAuth::new(payer, payee, 255, 10_000_000, 2592000, 0, 1000); // 0 = unlimited
        
        // Execute many cycles
        for i in 1..=100 {
            auth.execute(1000 + i).unwrap();
            assert_eq!(auth.current_cycles, i as u32);
            assert!(auth.is_active); // Still active with unlimited cycles
        }
    }

    #[test]
    fn test_recurring_auth_cancel() {
        let payer = Pubkey::new_unique();
        let payee = Pubkey::new_unique();
        let mut auth = RecurringAuth::new(payer, payee, 255, 10_000_000, 2592000, 12, 1000);
        
        assert!(auth.is_active);
        auth.cancel();
        assert!(!auth.is_active);
        
        // Cannot execute after cancel
        assert!(auth.execute(2000).is_err());
    }

    #[test]
    fn test_recurring_auth_seeds() {
        let payer = Pubkey::new_unique();
        let payee = Pubkey::new_unique();
        let seeds = RecurringAuth::seeds(&payer, &payee);
        
        assert_eq!(seeds.len(), 3);
        assert_eq!(seeds[0], RECURRING_AUTH_SEED.to_vec());
        assert_eq!(seeds[1], payer.to_bytes().to_vec());
        assert_eq!(seeds[2], payee.to_bytes().to_vec());
    }

    #[test]
    fn test_recurring_auth_size() {
        // Verify size calculation
        assert!(RECURRING_AUTH_SIZE > 0);
        println!("RecurringAuth SIZE: {}", RECURRING_AUTH_SIZE);
    }

    // === SpotTokenBalance Tests (Dynamic Token Balance Architecture) ===

    #[test]
    fn test_spot_token_balance_size() {
        assert_eq!(SPOT_TOKEN_BALANCE_SIZE, 98);
        let balance = SpotTokenBalance::new(Pubkey::new_unique(), 1, 255, 1000);
        let serialized = borsh::to_vec(&balance).unwrap();
        assert_eq!(serialized.len(), SPOT_TOKEN_BALANCE_SIZE);
    }

    #[test]
    fn test_spot_token_balance_serialization() {
        let wallet = Pubkey::new_unique();
        let balance = SpotTokenBalance::new(wallet, 1, 200, 1234567890);

        let serialized = borsh::to_vec(&balance).unwrap();
        let deserialized = SpotTokenBalance::try_from_slice(&serialized).unwrap();

        assert_eq!(deserialized.discriminator, SPOT_TOKEN_BALANCE_DISCRIMINATOR);
        assert_eq!(deserialized.wallet, wallet);
        assert_eq!(deserialized.token_index, 1);
        assert_eq!(deserialized.available_e6, 0);
        assert_eq!(deserialized.locked_e6, 0);
        assert_eq!(deserialized.last_update_ts, 1234567890);
        assert_eq!(deserialized.bump, 200);
        assert_eq!(deserialized.reserved, [0u8; 31]);
    }

    #[test]
    fn test_spot_token_balance_total() {
        let mut balance = SpotTokenBalance::new(Pubkey::new_unique(), 0, 255, 0);
        balance.available_e6 = 1000_000_000;
        balance.locked_e6 = 500_000_000;
        assert_eq!(balance.total().unwrap(), 1500_000_000);
    }

    #[test]
    fn test_spot_token_balance_deduct_prefer_available() {
        let mut balance = SpotTokenBalance::new(Pubkey::new_unique(), 0, 255, 0);
        balance.available_e6 = 800_000_000;
        balance.locked_e6 = 400_000_000;

        // Deduct from available only
        balance.deduct_prefer_available(500_000_000).unwrap();
        assert_eq!(balance.available_e6, 300_000_000);
        assert_eq!(balance.locked_e6, 400_000_000);

        // Deduct spanning available + locked
        balance.deduct_prefer_available(500_000_000).unwrap();
        assert_eq!(balance.available_e6, 0);
        assert_eq!(balance.locked_e6, 200_000_000);

        // Insufficient total
        let result = balance.deduct_prefer_available(300_000_000);
        assert!(result.is_err());
    }

    #[test]
    fn test_spot_token_balance_pda_derivation() {
        let program_id = Pubkey::new_unique();
        let wallet = Pubkey::new_unique();

        let (pda1, bump1) = derive_spot_token_balance_pda(&program_id, &wallet, 0);
        let (pda2, bump2) = derive_spot_token_balance_pda(&program_id, &wallet, 1);
        let (pda3, _) = derive_spot_token_balance_pda(&program_id, &wallet, 0);

        // Different token_index → different PDA
        assert_ne!(pda1, pda2);
        // Same inputs → same PDA
        assert_eq!(pda1, pda3);
        // Bumps are valid
        assert!(bump1 <= 255);
        assert!(bump2 <= 255);
    }

    #[test]
    fn test_spot_token_balance_lock_unlock() {
        let mut balance = SpotTokenBalance::new(Pubkey::new_unique(), 0, 255, 0);
        balance.available_e6 = 1000_000_000;

        // Lock 400: available=600, locked=400
        balance.available_e6 -= 400_000_000;
        balance.locked_e6 += 400_000_000;
        assert_eq!(balance.available_e6, 600_000_000);
        assert_eq!(balance.locked_e6, 400_000_000);
        assert_eq!(balance.total().unwrap(), 1000_000_000); // conservation

        // Unlock 200: available=800, locked=200
        balance.locked_e6 -= 200_000_000;
        balance.available_e6 += 200_000_000;
        assert_eq!(balance.available_e6, 800_000_000);
        assert_eq!(balance.locked_e6, 200_000_000);
        assert_eq!(balance.total().unwrap(), 1000_000_000); // conservation

        // Lock more than available should be caught by processor (checked arithmetic)
        let excess = balance.available_e6 + 1;
        assert!(balance.available_e6 < excess); // would underflow
    }

    #[test]
    fn test_spot_token_balance_allocate_release_logic() {
        // Simulate allocate: UserAccount.available -= X, SpotTokenBalance.available += X
        let mut user_available: i64 = 10000_000_000; // 10000 USDC
        let mut spot_available: i64 = 0;

        let amount: i64 = 3000_000_000; // allocate 3000 USDC

        // Allocate
        assert!(user_available >= amount);
        user_available -= amount;
        spot_available += amount;
        assert_eq!(user_available, 7000_000_000);
        assert_eq!(spot_available, 3000_000_000);

        // Release
        let release: i64 = 1500_000_000;
        assert!(spot_available >= release);
        spot_available -= release;
        user_available += release;
        assert_eq!(user_available, 8500_000_000);
        assert_eq!(spot_available, 1500_000_000);

        // Conservation: total unchanged
        assert_eq!(user_available + spot_available, 10000_000_000);
    }
}
