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
/// 
/// ⚠️ 重要：此结构必须与链上已部署的账户数据格式完全匹配！
/// 链上账户大小: 569 bytes
///
/// 修复记录 (2025-12-10):
/// - authorized_callers 从 Vec<Pubkey> 改为 [Pubkey; 10] 固定大小数组
/// - fund_program 从 Option<Pubkey> 改为 Pubkey
pub const VAULT_CONFIG_SIZE: usize = 8 + // discriminator
    32 + // admin
    32 + // usdc_mint
    32 + // vault_token_account
    32 * 10 + // authorized_callers ([Pubkey; 10])
    32 + // ledger_program
    32 + // fund_program (Pubkey，不是 Option)
    32 + // delegation_program
    8 + // total_deposits
    8 + // total_locked
    1 + // is_paused
    32; // 预留空间
// Total: 8 + 32 + 32 + 32 + 320 + 32 + 32 + 32 + 8 + 8 + 1 + 32 = 569 bytes ✓

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
/// 
/// ⚠️ 重要：此结构必须与链上已部署的账户数据格式完全匹配！
/// 链上账户大小: 569 bytes
///
/// 修复记录 (2025-12-10):
/// - authorized_callers 从 Vec<Pubkey> 改为 [Pubkey; 10] 固定大小数组
/// - fund_program 从 Option<Pubkey> 改为 Pubkey
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
    /// ⚠️ 固定大小数组，不是 Vec！
    pub authorized_callers: [Pubkey; 10],
    
    /// Ledger Program ID (32 bytes)
    pub ledger_program: Pubkey,
    
    /// Fund Program ID (32 bytes)
    /// ⚠️ 不是 Option<Pubkey>，链上是直接的 Pubkey
    pub fund_program: Pubkey,
    
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
// Total: 8 + 32 + 32 + 32 + 320 + 32 + 32 + 32 + 8 + 8 + 1 + 32 = 569 bytes ✓

impl VaultConfig {
    pub const DISCRIMINATOR: u64 = 0x5641554C545F434F; // "VAULT_CO"
    
    /// 验证调用方是否授权
    pub fn is_authorized_caller(&self, caller: &Pubkey) -> bool {
        // Check ledger_program
        if caller == &self.ledger_program {
            return true;
        }
        // Check fund_program (non-zero check)
        if self.fund_program != Pubkey::default() && caller == &self.fund_program {
            return true;
        }
        // Check authorized_callers array (skip zero pubkeys)
        for authorized in &self.authorized_callers {
            if authorized != &Pubkey::default() && caller == authorized {
                return true;
            }
        }
        false
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

// =============================================================================
// Spot 交易专用账户 (Phase 2/3: Spot Market Support)
// =============================================================================

/// SpotUserAccount discriminator
pub const SPOT_USER_ACCOUNT_DISCRIMINATOR: u64 = 0x53504F545F555352; // "SPOT_USR"

/// SpotUserAccount PDA seed
pub const SPOT_USER_SEED: &[u8] = b"spot_user";

/// 单个 Token 余额结构 (32 bytes)
/// token_index (2) + available (8) + locked (8) + reserved (14) = 32 bytes
pub const TOKEN_BALANCE_SIZE: usize = 32;

/// 最大支持的 Token 数量 (减少到16以避免栈溢出)
/// 用户若需要更多Token，可使用分页PDA: ["spot_user", wallet, page_index]
pub const MAX_TOKEN_SLOTS: usize = 16;

/// SpotUserAccount 账户大小 (bytes)
/// discriminator (8) + wallet (32) + bump (1) + last_settled_sequence (8) + 
/// token_count (2) + token_balances (16 * 32) + last_update_ts (8) + reserved (64) = 635 bytes
pub const SPOT_USER_ACCOUNT_SIZE: usize = 8 + 32 + 1 + 8 + 2 + (MAX_TOKEN_SLOTS * TOKEN_BALANCE_SIZE) + 8 + 64;

/// Token 余额结构
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone, Copy, Default)]
pub struct TokenBalance {
    /// Token 索引 (来自 Listing Program TokenRegistry)
    pub token_index: u16,
    /// 可用余额 (e6)
    pub available_e6: i64,
    /// 挂单锁定余额 (e6)
    pub locked_e6: i64,
    /// 预留空间
    pub reserved: [u8; 14],
}

impl TokenBalance {
    /// 判断槽位是否为空 (token_index == 0 且余额都为 0)
    pub fn is_empty(&self) -> bool {
        self.token_index == 0 && self.available_e6 == 0 && self.locked_e6 == 0
    }
    
    /// 总余额
    pub fn total(&self) -> i64 {
        self.available_e6 + self.locked_e6
    }
}

/// Spot 用户账户 (PDA)
/// Seeds: ["spot_user", wallet.key()]
/// 
/// 记录用户持有的多种 Token 余额，用于 Spot 交易
/// 独立于 Perp 的 UserAccount，避免相互干扰
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone)]
pub struct SpotUserAccount {
    /// 账户类型标识符
    pub discriminator: u64,
    
    /// 用户钱包地址
    pub wallet: Pubkey,
    
    /// PDA bump
    pub bump: u8,
    
    /// 最后结算序列号 (用于并发控制)
    pub last_settled_sequence: u64,
    
    /// 当前已使用的 Token 槽位数量
    pub token_count: u16,
    
    /// Token 余额数组 (最多 64 种)
    pub token_balances: [TokenBalance; MAX_TOKEN_SLOTS],
    
    /// 最后更新时间戳
    pub last_update_ts: i64,
    
    /// 预留字段
    pub reserved: [u8; 64],
}

impl SpotUserAccount {
    pub const DISCRIMINATOR: u64 = SPOT_USER_ACCOUNT_DISCRIMINATOR;
    
    /// PDA seeds
    pub fn seeds(wallet: &Pubkey) -> Vec<Vec<u8>> {
        vec![
            SPOT_USER_SEED.to_vec(),
            wallet.to_bytes().to_vec(),
        ]
    }
    
    /// 创建新的 Spot 用户账户
    pub fn new(wallet: Pubkey, bump: u8, created_at: i64) -> Self {
        Self {
            discriminator: Self::DISCRIMINATOR,
            wallet,
            bump,
            last_settled_sequence: 0,
            token_count: 0,
            token_balances: [TokenBalance::default(); MAX_TOKEN_SLOTS],
            last_update_ts: created_at,
            reserved: [0u8; 64],
        }
    }
    
    /// 查找指定 Token 的余额槽位
    /// 返回槽位索引，如果不存在返回 None
    pub fn find_token_slot(&self, token_index: u16) -> Option<usize> {
        for i in 0..self.token_count as usize {
            if self.token_balances[i].token_index == token_index {
                return Some(i);
            }
        }
        None
    }
    
    /// 获取指定 Token 的余额，如果不存在返回 None
    pub fn get_token_balance(&self, token_index: u16) -> Option<&TokenBalance> {
        self.find_token_slot(token_index).map(|i| &self.token_balances[i])
    }
    
    /// 获取或创建 Token 余额槽位
    /// 返回槽位索引，如果槽位已满返回 None
    pub fn get_or_create_token_slot(&mut self, token_index: u16) -> Option<usize> {
        // 先查找现有槽位
        if let Some(slot) = self.find_token_slot(token_index) {
            return Some(slot);
        }
        
        // 检查是否还有空槽位
        if self.token_count as usize >= MAX_TOKEN_SLOTS {
            return None; // 槽位已满
        }
        
        // 创建新槽位
        let slot = self.token_count as usize;
        self.token_balances[slot].token_index = token_index;
        self.token_count += 1;
        Some(slot)
    }
    
    /// 入金指定 Token
    pub fn deposit(&mut self, token_index: u16, amount: i64, current_ts: i64) -> Result<(), &'static str> {
        if amount <= 0 {
            return Err("Deposit amount must be positive");
        }
        
        let slot = self.get_or_create_token_slot(token_index)
            .ok_or("Token slots full")?;
        
        self.token_balances[slot].available_e6 = self.token_balances[slot].available_e6
            .checked_add(amount)
            .ok_or("Overflow")?;
        self.last_update_ts = current_ts;
        Ok(())
    }
    
    /// 出金指定 Token
    pub fn withdraw(&mut self, token_index: u16, amount: i64, current_ts: i64) -> Result<(), &'static str> {
        if amount <= 0 {
            return Err("Withdraw amount must be positive");
        }
        
        let slot = self.find_token_slot(token_index)
            .ok_or("Token not found")?;
        
        if self.token_balances[slot].available_e6 < amount {
            return Err("Insufficient balance");
        }
        
        self.token_balances[slot].available_e6 -= amount;
        self.last_update_ts = current_ts;
        Ok(())
    }
    
    /// 锁定余额 (挂单时)
    pub fn lock_balance(&mut self, token_index: u16, amount: i64, current_ts: i64) -> Result<(), &'static str> {
        if amount <= 0 {
            return Err("Lock amount must be positive");
        }
        
        let slot = self.find_token_slot(token_index)
            .ok_or("Token not found")?;
        
        if self.token_balances[slot].available_e6 < amount {
            return Err("Insufficient balance to lock");
        }
        
        self.token_balances[slot].available_e6 -= amount;
        self.token_balances[slot].locked_e6 = self.token_balances[slot].locked_e6
            .checked_add(amount)
            .ok_or("Overflow")?;
        self.last_update_ts = current_ts;
        Ok(())
    }
    
    /// 解锁余额 (撤单时)
    pub fn unlock_balance(&mut self, token_index: u16, amount: i64, current_ts: i64) -> Result<(), &'static str> {
        if amount <= 0 {
            return Err("Unlock amount must be positive");
        }
        
        let slot = self.find_token_slot(token_index)
            .ok_or("Token not found")?;
        
        if self.token_balances[slot].locked_e6 < amount {
            return Err("Insufficient locked balance");
        }
        
        self.token_balances[slot].locked_e6 -= amount;
        self.token_balances[slot].available_e6 = self.token_balances[slot].available_e6
            .checked_add(amount)
            .ok_or("Overflow")?;
        self.last_update_ts = current_ts;
        Ok(())
    }
    
    /// Spot 交易结算
    /// 
    /// Buy 方: base_token 增加, quote_token 减少 (从 locked)
    /// Sell 方: base_token 减少 (从 locked), quote_token 增加
    pub fn settle_trade(
        &mut self,
        is_buy: bool,
        base_token_index: u16,
        quote_token_index: u16,
        base_amount: i64,
        quote_amount: i64,
        sequence: u64,
        current_ts: i64,
    ) -> Result<(), &'static str> {
        // 检查序列号 (防止重复结算)
        if sequence <= self.last_settled_sequence {
            return Err("Invalid sequence");
        }
        
        if is_buy {
            // Buy: 支付 quote_token (从 locked), 获得 base_token
            let quote_slot = self.find_token_slot(quote_token_index)
                .ok_or("Quote token not found")?;
            
            if self.token_balances[quote_slot].locked_e6 < quote_amount {
                return Err("Insufficient locked quote balance");
            }
            self.token_balances[quote_slot].locked_e6 -= quote_amount;
            
            // 增加 base_token
            let base_slot = self.get_or_create_token_slot(base_token_index)
                .ok_or("Token slots full")?;
            self.token_balances[base_slot].available_e6 = self.token_balances[base_slot].available_e6
                .checked_add(base_amount)
                .ok_or("Overflow")?;
        } else {
            // Sell: 支付 base_token (从 locked), 获得 quote_token
            let base_slot = self.find_token_slot(base_token_index)
                .ok_or("Base token not found")?;
            
            if self.token_balances[base_slot].locked_e6 < base_amount {
                return Err("Insufficient locked base balance");
            }
            self.token_balances[base_slot].locked_e6 -= base_amount;
            
            // 增加 quote_token
            let quote_slot = self.get_or_create_token_slot(quote_token_index)
                .ok_or("Token slots full")?;
            self.token_balances[quote_slot].available_e6 = self.token_balances[quote_slot].available_e6
                .checked_add(quote_amount)
                .ok_or("Overflow")?;
        }
        
        self.last_settled_sequence = sequence;
        self.last_update_ts = current_ts;
        Ok(())
    }
    
    /// Spot 交易结算 V2 (2025-12-31 新增)
    /// 
    /// 优先从 available_e6 扣除，符合 Hyperliquid 模式：
    /// - 链下验证余额
    /// - 链上只结算，不需要预先锁定
    /// 
    /// Buy 方: base_token 增加, quote_token 减少 (从 available - fee)
    /// Sell 方: base_token 减少 (从 available), quote_token 增加 (- fee)
    pub fn settle_trade_v2(
        &mut self,
        is_buy: bool,
        base_token_index: u16,
        quote_token_index: u16,
        base_amount: i64,
        quote_amount: i64,
        fee_e6: i64,
        sequence: u64,
        current_ts: i64,
    ) -> Result<(), &'static str> {
        // 检查序列号 (防止重复结算)
        if sequence <= self.last_settled_sequence {
            return Err("Invalid sequence - already settled");
        }
        
        if is_buy {
            // Buy: 支付 quote_token + fee, 获得 base_token
            let quote_slot = self.get_or_create_token_slot(quote_token_index)
                .ok_or("Token slots full")?;
            
            let total_cost = quote_amount + fee_e6;
            
            // 优先从 available 扣除，不足则从 locked 扣除
            let available = self.token_balances[quote_slot].available_e6;
            let locked = self.token_balances[quote_slot].locked_e6;
            
            if available >= total_cost {
                // 全部从 available 扣除
                self.token_balances[quote_slot].available_e6 -= total_cost;
            } else if available + locked >= total_cost {
                // 先扣 available，不足从 locked 补
                let from_locked = total_cost - available;
                self.token_balances[quote_slot].available_e6 = 0;
                self.token_balances[quote_slot].locked_e6 -= from_locked;
            } else {
                return Err("Insufficient quote balance for buy");
            }
            
            // 增加 base_token
            let base_slot = self.get_or_create_token_slot(base_token_index)
                .ok_or("Token slots full")?;
            self.token_balances[base_slot].available_e6 = self.token_balances[base_slot].available_e6
                .checked_add(base_amount)
                .ok_or("Overflow")?;
        } else {
            // Sell: 支付 base_token, 获得 quote_token - fee
            let base_slot = self.get_or_create_token_slot(base_token_index)
                .ok_or("Token slots full")?;
            
            // 优先从 available 扣除，不足则从 locked 扣除
            let available = self.token_balances[base_slot].available_e6;
            let locked = self.token_balances[base_slot].locked_e6;
            
            if available >= base_amount {
                self.token_balances[base_slot].available_e6 -= base_amount;
            } else if available + locked >= base_amount {
                let from_locked = base_amount - available;
                self.token_balances[base_slot].available_e6 = 0;
                self.token_balances[base_slot].locked_e6 -= from_locked;
            } else {
                return Err("Insufficient base balance for sell");
            }
            
            // 增加 quote_token (扣除手续费)
            let quote_slot = self.get_or_create_token_slot(quote_token_index)
                .ok_or("Token slots full")?;
            let net_quote = quote_amount - fee_e6;
            if net_quote < 0 {
                return Err("Fee exceeds quote amount");
            }
            self.token_balances[quote_slot].available_e6 = self.token_balances[quote_slot].available_e6
                .checked_add(net_quote)
                .ok_or("Overflow")?;
        }
        
        self.last_settled_sequence = sequence;
        self.last_update_ts = current_ts;
        Ok(())
    }
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
    
    /// 更新状态哈希
    pub fn update_state_hash(&mut self, hash: [u8; 32]) {
        self.state_hash = hash;
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
        
        // Create authorized_callers array with the authorized key
        let mut authorized_callers = [Pubkey::default(); 10];
        authorized_callers[0] = authorized;
        
        let config = VaultConfig {
            discriminator: VaultConfig::DISCRIMINATOR,
            admin: Pubkey::new_unique(),
            usdc_mint: Pubkey::new_unique(),
            vault_token_account: Pubkey::new_unique(),
            authorized_callers,
            ledger_program: ledger,
            fund_program: fund,
            delegation_program: Pubkey::new_unique(),
            total_deposits: 0,
            total_locked: 0,
            is_paused: false,
            reserved: [0u8; 32],
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
}
