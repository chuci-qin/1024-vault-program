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

    /// 清算用户账户 (CPI only)
    /// 
    /// 执行清算时的资金处理:
    /// 1. 清空用户锁定保证金
    /// 2. 返还剩余给用户
    /// 3. 将清算罚金转入 Insurance Fund (实际 Token Transfer)
    /// 
    /// Accounts:
    /// 0. `[]` VaultConfig
    /// 1. `[writable]` UserAccount
    /// 2. `[]` Caller Program
    /// 3. `[writable]` Vault Token Account (源账户)
    /// 4. `[writable]` Insurance Fund Vault (目标账户 - Fund Program)
    /// 5. `[]` Token Program
    LiquidatePosition {
        /// 用户锁定的保证金 (e6) - 将被清空
        margin: u64,
        /// 返还给用户的剩余 (e6)
        user_remainder: u64,
        /// 清算罚金 (e6) - 转入 Insurance Fund
        liquidation_penalty: u64,
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

    /// Admin 强制释放用户锁定保证金 (Admin only)
    /// 
    /// 用于处理用户没有任何持仓但 locked_margin 残留的异常情况
    /// 
    /// Accounts:
    /// 0. `[signer]` Admin
    /// 1. `[writable]` UserAccount PDA
    /// 2. `[]` VaultConfig
    AdminForceReleaseMargin {
        /// 要释放的金额 (e6)，如果为 0 则释放全部 locked_margin
        amount: u64,
    },
    
    // =========================================================================
    // Prediction Market 预测市场相关指令 - 使用独立的 PredictionMarketUserAccount PDA
    // =========================================================================

    /// 初始化预测市场用户账户
    /// 
    /// 创建独立的 PredictionMarketUserAccount PDA，不修改现有 UserAccount
    /// 
    /// Accounts:
    /// 0. `[signer]` User
    /// 1. `[writable]` PredictionMarketUserAccount PDA
    /// 2. `[]` System Program
    InitializePredictionMarketUser,

    /// 预测市场锁定 USDC (CPI only - 由 Prediction Market Program 调用)
    /// 
    /// 流程:
    /// 1. 从 UserAccount.available_balance 扣除金额
    /// 2. 增加 PredictionMarketUserAccount.prediction_market_locked
    /// 
    /// Accounts:
    /// 0. `[]` VaultConfig
    /// 1. `[writable]` UserAccount (扣除 available_balance)
    /// 2. `[writable]` PredictionMarketUserAccount (增加 prediction_market_locked)
    /// 3. `[]` Caller Program (验证白名单)
    PredictionMarketLock {
        /// 锁定金额 (e6)
        amount: u64,
    },

    /// 预测市场释放锁定 (CPI only)
    /// 
    /// 用户卖出 YES/NO Token 或赎回完整集时
    /// 
    /// 流程:
    /// 1. 从 PredictionMarketUserAccount.prediction_market_locked 扣除
    /// 2. 增加 UserAccount.available_balance
    /// 
    /// Accounts:
    /// 0. `[]` VaultConfig
    /// 1. `[writable]` UserAccount
    /// 2. `[writable]` PredictionMarketUserAccount
    /// 3. `[]` Caller Program (验证白名单)
    PredictionMarketUnlock {
        /// 释放金额 (e6)
        amount: u64,
    },

    /// 预测市场结算 (CPI only)
    /// 
    /// 市场结算后，释放锁定并记录应得的结算金额
    /// 
    /// 流程:
    /// 1. 从 PredictionMarketUserAccount.prediction_market_locked 扣除 locked_amount
    /// 2. 将 settlement_amount 记入 PredictionMarketUserAccount.prediction_market_pending_settlement
    /// 
    /// Accounts:
    /// 0. `[]` VaultConfig
    /// 1. `[writable]` PredictionMarketUserAccount
    /// 2. `[]` Caller Program
    PredictionMarketSettle {
        /// 用户原锁定金额 (e6)
        locked_amount: u64,
        /// 结算应得金额 (e6)
        settlement_amount: u64,
    },

    /// 预测市场领取结算收益
    /// 
    /// 用户主动调用，将 prediction_market_pending_settlement 转为 UserAccount.available_balance
    /// 
    /// Accounts:
    /// 0. `[signer]` User
    /// 1. `[writable]` UserAccount
    /// 2. `[writable]` PredictionMarketUserAccount
    PredictionMarketClaimSettlement,

    /// Admin 强制释放预测市场锁定 (Admin only)
    /// 
    /// 用于处理异常情况（如市场取消后用户未操作）
    /// 
    /// Accounts:
    /// 0. `[signer]` Admin
    /// 1. `[writable]` UserAccount
    /// 2. `[writable]` PredictionMarketUserAccount
    /// 3. `[]` VaultConfig
    AdminPredictionMarketForceUnlock {
        /// 要释放的金额 (e6)，如果为 0 则释放全部
        amount: u64,
    },

    // =========================================================================
    // Relayer 指令 - 用于跨链入金/出金
    // =========================================================================

    /// Relayer 代理入金 (Admin/Relayer only)
    /// 
    /// 用途：当用户在 Solana 主网/Arbitrum 等链转账后，
    /// 由授权的 Relayer 代替用户在 1024Chain 上入金到 Vault
    /// 
    /// 特性：
    /// - 如果用户 UserAccount 不存在，会自动创建
    /// - 仅 Admin 可调用 (测试网自由入金)
    /// - 不涉及实际 Token Transfer（余额凭证模式）
    /// 
    /// Accounts:
    /// 0. `[signer]` Admin/Relayer
    /// 1. `[writable]` UserAccount PDA (会自动创建)
    /// 2. `[writable]` VaultConfig
    /// 3. `[]` System Program (用于创建账户)
    RelayerDeposit {
        /// 目标用户钱包地址
        user_wallet: Pubkey,
        /// 入金金额 (e6)
        amount: u64,
    },

    /// Relayer 代理出金 (Admin/Relayer only)
    /// 
    /// 用途：用户请求出金后，Relayer 在 1024Chain 上扣除余额，
    /// 然后在 Solana 主网/Arbitrum 等链上给用户转账
    /// 
    /// 安全性：
    /// - 仅 Admin 可调用
    /// - 必须验证用户有足够余额
    /// - 出金后 Relayer 负责在对应链完成转账
    /// 
    /// Accounts:
    /// 0. `[signer]` Admin/Relayer
    /// 1. `[writable]` UserAccount PDA
    /// 2. `[]` VaultConfig
    RelayerWithdraw {
        /// 目标用户钱包地址
        user_wallet: Pubkey,
        /// 出金金额 (e6)
        amount: u64,
    },
}

