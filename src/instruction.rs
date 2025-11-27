//! Vault Program Instructions
//!
//! Vault Program 职责: 纯用户资金托管
//! 保险基金相关操作已迁移到 Fund Program

use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::pubkey::Pubkey;

/// Vault Program 指令
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone)]
pub enum VaultInstruction {
    /// 初始化 Vault 配置
    /// 
    /// Accounts:
    /// 0. `[writable, signer]` Admin
    /// 1. `[writable]` VaultConfig PDA
    /// 2. `[]` USDC Mint
    /// 3. `[writable]` Vault Token Account
    /// 4. `[]` System Program
    /// 5. `[]` Token Program
    /// 6. `[]` Rent Sysvar
    Initialize {
        /// Ledger Program ID
        ledger_program: Pubkey,
        /// Delegation Program ID
        delegation_program: Pubkey,
        /// Fund Program ID (用于 CPI 调用保险基金等)
        fund_program: Pubkey,
    },

    /// 初始化用户账户
    /// 
    /// Accounts:
    /// 0. `[signer]` User
    /// 1. `[writable]` UserAccount PDA
    /// 2. `[]` System Program
    InitializeUser,

    /// 入金
    /// 
    /// Accounts:
    /// 0. `[signer]` User
    /// 1. `[writable]` UserAccount PDA
    /// 2. `[writable]` User USDC Token Account
    /// 3. `[writable]` Vault USDC Token Account
    /// 4. `[writable]` VaultConfig
    /// 5. `[]` Token Program
    Deposit {
        /// 存款金额 (e6)
        amount: u64,
    },

    /// 出金
    /// 
    /// Accounts:
    /// 0. `[signer]` User
    /// 1. `[writable]` UserAccount PDA
    /// 2. `[writable]` User USDC Token Account
    /// 3. `[writable]` Vault USDC Token Account
    /// 4. `[]` VaultConfig
    /// 5. `[]` Token Program
    Withdraw {
        /// 提款金额 (e6)
        amount: u64,
    },

    /// 锁定保证金 (CPI only)
    /// 
    /// Accounts:
    /// 0. `[]` VaultConfig
    /// 1. `[writable]` UserAccount
    /// 2. `[]` Caller Program (验证白名单)
    LockMargin {
        /// 锁定金额 (e6)
        amount: u64,
    },

    /// 释放保证金 (CPI only)
    /// 
    /// Accounts:
    /// 0. `[]` VaultConfig
    /// 1. `[writable]` UserAccount
    /// 2. `[]` Caller Program (验证白名单)
    ReleaseMargin {
        /// 释放金额 (e6)
        amount: u64,
    },

    /// 平仓结算 (CPI only - 合并操作)
    /// 
    /// 注意: 手续费收取由 Ledger Program 单独通过 CPI 调用 Fund Program
    /// 
    /// Accounts:
    /// 0. `[]` VaultConfig
    /// 1. `[writable]` UserAccount
    /// 2. `[]` Caller Program
    ClosePositionSettle {
        /// 释放的保证金 (e6)
        margin_to_release: u64,
        /// 实现盈亏 (e6, 正=盈利, 负=亏损)
        realized_pnl: i64,
        /// 手续费 (e6) - 从用户余额扣除
        fee: u64,
    },

    /// 清算用户账户部分 (CPI only)
    /// 
    /// 注意: 清算罚金/穿仓由 Ledger Program 单独通过 CPI 调用 Fund Program
    /// 此指令仅处理用户账户的余额更新
    /// 
    /// Accounts:
    /// 0. `[]` VaultConfig
    /// 1. `[writable]` UserAccount
    /// 2. `[]` Caller Program
    LiquidatePosition {
        /// 用户锁定的保证金 (e6) - 将被清空
        margin: u64,
        /// 返还给用户的剩余 (e6)
        user_remainder: u64,
    },

    /// 添加授权调用方 (Admin only)
    /// 
    /// Accounts:
    /// 0. `[signer]` Admin
    /// 1. `[writable]` VaultConfig
    AddAuthorizedCaller {
        /// 新的授权调用方
        caller: Pubkey,
    },

    /// 移除授权调用方 (Admin only)
    /// 
    /// Accounts:
    /// 0. `[signer]` Admin
    /// 1. `[writable]` VaultConfig
    RemoveAuthorizedCaller {
        /// 要移除的调用方
        caller: Pubkey,
    },

    /// 暂停/恢复 (Admin only)
    /// 
    /// Accounts:
    /// 0. `[signer]` Admin
    /// 1. `[writable]` VaultConfig
    SetPaused {
        /// 是否暂停
        paused: bool,
    },

    /// 更新管理员 (Admin only)
    /// 
    /// Accounts:
    /// 0. `[signer]` Current Admin
    /// 1. `[writable]` VaultConfig
    UpdateAdmin {
        /// 新管理员
        new_admin: Pubkey,
    },
    
    /// 设置 Fund Program (Admin only)
    /// 
    /// Accounts:
    /// 0. `[signer]` Admin
    /// 1. `[writable]` VaultConfig
    SetFundProgram {
        /// Fund Program ID
        fund_program: Pubkey,
    },
}
