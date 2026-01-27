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

    /// 设置 Ledger Program (Admin only)
    /// 
    /// Accounts:
    /// 0. `[signer]` Admin
    /// 1. `[writable]` VaultConfig
    SetLedgerProgram {
        /// Ledger Program ID
        ledger_program: Pubkey,
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

    /// 预测市场锁定 USDC 并扣除手续费 (CPI only - 由 Prediction Market Program 调用)
    /// 
    /// V2 Fee Architecture: 在 Vault 层面收取手续费
    /// 
    /// 流程:
    /// 1. 从 UserAccount.available_balance 扣除 gross_amount
    /// 2. 读取 PM Fee Config 获取 minting_fee_bps
    /// 3. 计算 fee = gross_amount * fee_bps / 10000
    /// 4. net_amount = gross_amount - fee
    /// 5. 增加 PredictionMarketUserAccount.prediction_market_locked += net_amount
    /// 6. spl_token::transfer(Vault Token Account → PM Fee Vault, fee)
    /// 7. 更新 PM Fee Config 统计
    /// 
    /// Accounts:
    /// 0. `[]` VaultConfig
    /// 1. `[writable]` UserAccount (扣除 available_balance)
    /// 2. `[writable]` PredictionMarketUserAccount (增加 prediction_market_locked)
    /// 3. `[]` Caller Program (验证白名单)
    /// 4. `[writable]` Vault Token Account (源账户)
    /// 5. `[writable]` PM Fee Vault (目标账户)
    /// 6. `[writable]` PM Fee Config PDA (更新统计)
    /// 7. `[]` Token Program
    /// 8. `[signer, writable]` Payer (for auto-init, optional)
    /// 9. `[]` System Program (for auto-init, optional)
    PredictionMarketLockWithFee {
        /// 用户输入的总金额 (e6)，包含手续费
        gross_amount: u64,
    },

    /// 预测市场释放锁定并扣除手续费 (CPI only)
    /// 
    /// V2 Fee Architecture: 在 Vault 层面收取赎回手续费
    /// 
    /// 流程:
    /// 1. 从 PredictionMarketUserAccount.prediction_market_locked 扣除 gross_amount
    /// 2. 读取 PM Fee Config 获取 redemption_fee_bps
    /// 3. 计算 fee = gross_amount * fee_bps / 10000
    /// 4. net_amount = gross_amount - fee
    /// 5. 增加 UserAccount.available_balance += net_amount
    /// 6. spl_token::transfer(Vault Token Account → PM Fee Vault, fee)
    /// 7. 更新 PM Fee Config 统计
    /// 
    /// Accounts:
    /// 0. `[]` VaultConfig
    /// 1. `[writable]` UserAccount
    /// 2. `[writable]` PredictionMarketUserAccount
    /// 3. `[]` Caller Program
    /// 4. `[writable]` Vault Token Account
    /// 5. `[writable]` PM Fee Vault
    /// 6. `[writable]` PM Fee Config PDA
    /// 7. `[]` Token Program
    PredictionMarketUnlockWithFee {
        /// 要释放的金额 (e6)
        gross_amount: u64,
    },

    /// 预测市场交易费收取 (CPI only - 由 Prediction Market Program 调用)
    /// 
    /// V2 Fee Architecture: 交易撮合时收取 Taker/Maker 费用
    /// 
    /// 流程:
    /// 1. 读取 PM Fee Config 获取 taker_fee_bps / maker_fee_bps
    /// 2. 计算 taker_fee = trade_amount * taker_fee_bps / 10000
    /// 3. 计算 maker_fee = trade_amount * maker_fee_bps / 10000 (可为0或负数表示返佣)
    /// 4. 从 Vault Token Account 转账 (taker_fee + maker_fee) → PM Fee Vault
    /// 5. 更新 PM Fee Config 统计 (total_trading_fee)
    /// 
    /// 注意: 此指令仅收取费用，不修改用户余额。余额调整由 PM Program 在调用前完成。
    /// 
    /// Accounts:
    /// 0. `[]` VaultConfig
    /// 1. `[]` Caller Program (验证白名单)
    /// 2. `[writable]` Vault Token Account (源账户)
    /// 3. `[writable]` PM Fee Vault (目标账户)
    /// 4. `[writable]` PM Fee Config PDA (更新统计)
    /// 5. `[]` Token Program
    PredictionMarketTradeWithFee {
        /// 交易金额 (e6)，用于计算费用
        trade_amount: u64,
        /// 是否为 Taker (true=Taker, false=Maker)
        is_taker: bool,
    },

    /// 预测市场结算并扣除手续费 (CPI only)
    /// 
    /// V2 Fee Architecture: 市场结算时收取结算费
    /// 
    /// 流程:
    /// 1. 从 PredictionMarketUserAccount.prediction_market_locked 扣除 locked_amount
    /// 2. 读取 PM Fee Config 获取 settlement_fee_bps
    /// 3. 计算 fee = settlement_amount * fee_bps / 10000
    /// 4. net_settlement = settlement_amount - fee
    /// 5. 将 net_settlement 记入 PredictionMarketUserAccount.prediction_market_pending_settlement
    /// 6. spl_token::transfer(Vault Token Account → PM Fee Vault, fee)
    /// 7. 更新 PM Fee Config 统计
    /// 
    /// Accounts:
    /// 0. `[]` VaultConfig
    /// 1. `[writable]` PredictionMarketUserAccount
    /// 2. `[]` Caller Program
    /// 3. `[writable]` Vault Token Account (源账户)
    /// 4. `[writable]` PM Fee Vault (目标账户)
    /// 5. `[writable]` PM Fee Config PDA
    /// 6. `[]` Token Program
    PredictionMarketSettleWithFee {
        /// 用户原锁定金额 (e6)
        locked_amount: u64,
        /// 结算应得金额 (e6)，扣除手续费后入账
        settlement_amount: u64,
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

    // =========================================================================
    // Spot 交易相关指令 - 使用独立的 SpotUserAccount PDA
    // =========================================================================

    /// 初始化 Spot 用户账户
    /// 
    /// 创建 SpotUserAccount PDA，支持多种 Token 余额管理
    /// 
    /// Accounts:
    /// 0. `[signer]` User (payer)
    /// 1. `[writable]` SpotUserAccount PDA
    /// 2. `[]` System Program
    InitializeSpotUser,

    /// Spot Token 入金 (用户直接调用)
    /// 
    /// 将用户的 SPL Token 转入 Vault，增加 SpotUserAccount 余额
    /// 
    /// Accounts:
    /// 0. `[signer]` User
    /// 1. `[writable]` SpotUserAccount PDA
    /// 2. `[writable]` User Token Account (SPL Token)
    /// 3. `[writable]` Vault Token Account (SPL Token)
    /// 4. `[]` VaultConfig
    /// 5. `[]` Token Program
    SpotDeposit {
        /// Token 索引 (来自 Listing Program TokenRegistry)
        token_index: u16,
        /// 入金金额 (e6 或原生精度)
        amount: u64,
    },

    /// Spot Token 出金 (用户直接调用)
    /// 
    /// 将 Vault 中的 Token 转回给用户
    /// 
    /// Accounts:
    /// 0. `[signer]` User
    /// 1. `[writable]` SpotUserAccount PDA
    /// 2. `[writable]` User Token Account (SPL Token)
    /// 3. `[writable]` Vault Token Account (SPL Token)
    /// 4. `[]` VaultConfig
    /// 5. `[]` Token Program
    SpotWithdraw {
        /// Token 索引
        token_index: u16,
        /// 出金金额
        amount: u64,
    },

    /// Spot 锁定余额 (CPI only - 挂单时)
    /// 
    /// 由 Matcher/Gateway 调用，锁定用户余额以防止超卖
    /// 
    /// Accounts:
    /// 0. `[]` VaultConfig
    /// 1. `[writable]` SpotUserAccount PDA
    /// 2. `[]` Caller Program (验证白名单)
    SpotLockBalance {
        /// Token 索引
        token_index: u16,
        /// 锁定金额
        amount: u64,
    },

    /// Spot 解锁余额 (CPI only - 撤单时)
    /// 
    /// 由 Matcher/Gateway 调用，释放锁定的余额
    /// 
    /// Accounts:
    /// 0. `[]` VaultConfig
    /// 1. `[writable]` SpotUserAccount PDA
    /// 2. `[]` Caller Program
    SpotUnlockBalance {
        /// Token 索引
        token_index: u16,
        /// 解锁金额
        amount: u64,
    },

    /// Spot 交易结算 (CPI only - 成交时)
    /// 
    /// 由 Relayer/Settlement 调用，执行 Token 余额变动
    /// 
    /// 流程 (Buy):
    /// 1. 从 buyer.locked[quote_token] 扣除 quote_amount
    /// 2. 增加 buyer.available[base_token] += base_amount
    /// 
    /// 流程 (Sell):
    /// 1. 从 seller.locked[base_token] 扣除 base_amount
    /// 2. 增加 seller.available[quote_token] += quote_amount
    /// 
    /// Accounts:
    /// 0. `[]` VaultConfig
    /// 1. `[writable]` SpotUserAccount PDA
    /// 2. `[]` Caller Program
    SpotSettleTrade {
        /// 是否为 Buy 方
        is_buy: bool,
        /// Base Token 索引
        base_token_index: u16,
        /// Quote Token 索引
        quote_token_index: u16,
        /// Base 数量
        base_amount: u64,
        /// Quote 数量
        quote_amount: u64,
        /// 序列号 (防止重复结算)
        sequence: u64,
    },

    /// Relayer 代理 Spot 入金 (Admin/Relayer only)
    /// 
    /// 类似 RelayerDeposit，用于跨链 Token 入金
    /// 
    /// Accounts:
    /// 0. `[signer]` Admin/Relayer
    /// 1. `[writable]` SpotUserAccount PDA (会自动创建)
    /// 2. `[writable]` VaultConfig
    /// 3. `[]` System Program
    RelayerSpotDeposit {
        /// 目标用户钱包地址
        user_wallet: Pubkey,
        /// Token 索引
        token_index: u16,
        /// 入金金额
        amount: u64,
    },

    /// Relayer 代理 Spot 出金 (Admin/Relayer only)
    /// 
    /// Accounts:
    /// 0. `[signer]` Admin/Relayer
    /// 1. `[writable]` SpotUserAccount PDA
    /// 2. `[]` VaultConfig
    RelayerSpotWithdraw {
        /// 目标用户钱包地址
        user_wallet: Pubkey,
        /// Token 索引
        token_index: u16,
        /// 出金金额
        amount: u64,
    },

    // =========================================================================
    // Spot 统一账户指令 (2025-12-31 新增)
    // =========================================================================

    /// Relayer 代理 Spot 交易结算 (Admin/Relayer only)
    /// 
    /// CEX 级体验：用户交易无需签名，由 Relayer 代理结算
    /// 同时更新 Maker 和 Taker 两个 SpotUserAccount
    /// 
    /// Accounts:
    /// 0. `[signer]` Admin/Relayer
    /// 1. `[writable]` Maker SpotUserAccount PDA
    /// 2. `[writable]` Taker SpotUserAccount PDA
    /// 3. `[]` VaultConfig
    RelayerSpotSettleTrade {
        /// Maker 钱包地址
        maker_wallet: Pubkey,
        /// Taker 钱包地址
        taker_wallet: Pubkey,
        /// Base Token 索引 (e.g., BTC=1)
        base_token_index: u16,
        /// Quote Token 索引 (e.g., USDC=0)
        quote_token_index: u16,
        /// Base 数量 (e6)
        base_amount_e6: i64,
        /// Quote 数量 (e6)
        quote_amount_e6: i64,
        /// Maker 手续费 (e6)
        maker_fee_e6: i64,
        /// Taker 手续费 (e6)
        taker_fee_e6: i64,
        /// Taker 是否为买方
        taker_is_buy: bool,
        /// 序列号 (防止重复结算)
        sequence: u64,
    },

    /// 从 UserAccount 划转 USDC 到 SpotUserAccount (Admin/Relayer only)
    /// 
    /// 统一账户体验：Spot 买入前，将 USDC 从主账户划转到 Spot 账户
    /// 
    /// Accounts:
    /// 0. `[signer]` Admin/Relayer
    /// 1. `[writable]` UserAccount PDA (seed: ["user", wallet])
    /// 2. `[writable]` SpotUserAccount PDA (seed: ["spot_user", wallet])
    /// 3. `[]` VaultConfig
    /// 4. `[]` System Program (用于自动创建 SpotUserAccount)
    SpotAllocateFromVault {
        /// 用户钱包地址
        user_wallet: Pubkey,
        /// 划转金额 (e6)
        amount: u64,
    },

    /// 从 SpotUserAccount 划转 USDC 到 UserAccount (Admin/Relayer only)
    /// 
    /// 统一账户体验：Spot 卖出后，将 USDC 从 Spot 账户划转回主账户
    /// 
    /// Accounts:
    /// 0. `[signer]` Admin/Relayer
    /// 1. `[writable]` SpotUserAccount PDA
    /// 2. `[writable]` UserAccount PDA
    /// 3. `[]` VaultConfig
    SpotReleaseToVault {
        /// 用户钱包地址
        user_wallet: Pubkey,
        /// 划转金额 (e6)
        amount: u64,
    },

    // =========================================================================
    // 站内支付相关指令 (2026-01-27 新增)
    // =========================================================================

    /// Relayer 代理内部转账 (站内支付)
    /// 
    /// 用于一次性支付和定时支付，从发送方 Vault 余额转账到接收方
    /// 同时收取固定手续费 (1 USDC) 进入 InsuranceFund
    /// 
    /// 安全性：
    /// - 仅 Admin/Relayer 可调用
    /// - 验证发送方有足够余额 (amount + fee)
    /// - 手续费进入 InsuranceFund
    /// 
    /// Accounts:
    /// 0. `[signer]` Admin/Relayer
    /// 1. `[writable]` From UserAccount PDA
    /// 2. `[writable]` To UserAccount PDA
    /// 3. `[]` VaultConfig
    /// 4. `[writable]` Insurance Fund (接收手续费)
    RelayerInternalTransfer {
        /// 发送方钱包地址
        from_wallet: Pubkey,
        /// 接收方钱包地址
        to_wallet: Pubkey,
        /// 转账金额 (e6)
        amount: u64,
        /// 手续费 (e6) = 1_000_000 (1 USDC)
        fee: u64,
        /// 转账类型: 0=onetime, 1=recurring, 2=registration
        transfer_type: u8,
        /// 业务关联哈希 (用于幂等性)
        reference_hash: [u8; 32],
    },

    /// 初始化定时支付授权 PDA (链上存证)
    /// 
    /// 创建 RecurringAuth PDA 账户，记录定时支付授权信息
    /// 同时收取注册手续费 (1 USDC)
    /// 
    /// Accounts:
    /// 0. `[signer]` Admin/Relayer
    /// 1. `[writable]` Payer UserAccount PDA
    /// 2. `[writable]` RecurringAuth PDA (新建)
    /// 3. `[]` VaultConfig
    /// 4. `[writable]` Insurance Fund (接收手续费)
    /// 5. `[]` System Program
    InitRecurringAuth {
        /// 付款方钱包
        payer: Pubkey,
        /// 收款方钱包
        payee: Pubkey,
        /// 每期金额 (e6)
        amount: u64,
        /// 扣款周期 (秒)
        interval_seconds: i64,
        /// 最大执行次数 (0=无限)
        max_cycles: u32,
        /// 注册手续费 (e6) = 1_000_000 (1 USDC)
        registration_fee: u64,
    },

    /// 执行定时支付扣款 (由 Scheduler 调用)
    /// 
    /// 从付款方扣除 amount + fee，转给收款方
    /// 更新 RecurringAuth PDA 的执行次数
    /// 
    /// Accounts:
    /// 0. `[signer]` Admin/Relayer
    /// 1. `[writable]` Payer UserAccount PDA
    /// 2. `[writable]` Payee UserAccount PDA
    /// 3. `[writable]` RecurringAuth PDA
    /// 4. `[]` VaultConfig
    /// 5. `[writable]` Insurance Fund (接收手续费)
    ExecuteRecurringPayment {
        /// 付款方钱包
        payer: Pubkey,
        /// 收款方钱包
        payee: Pubkey,
        /// 转账金额 (e6)
        amount: u64,
        /// 手续费 (e6) = 1_000_000 (1 USDC)
        fee: u64,
        /// 当前执行次数
        cycle_count: u32,
    },

    /// 取消定时支付授权
    /// 
    /// 标记 RecurringAuth PDA 为非激活状态
    /// 不收取手续费
    /// 
    /// Accounts:
    /// 0. `[signer]` Admin/Relayer
    /// 1. `[writable]` RecurringAuth PDA
    /// 2. `[]` VaultConfig
    CancelRecurringAuth {
        /// 付款方钱包
        payer: Pubkey,
        /// 收款方钱包
        payee: Pubkey,
    },
}

