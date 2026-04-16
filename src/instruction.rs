//! Vault Program Instructions
//!
//! 18 active instructions for user fund custody.
//! Only two on-chain programs remain: Vault (fund custody) + Exchange (on-chain audit).

use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::pubkey::Pubkey;

/// Vault Program 指令 (18 active variants)
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone)]
pub enum VaultInstruction {
    /// Index 0: 初始化 Vault 配置
    ///
    /// Accounts:
    /// 0. `[writable, signer]` Governance Authority
    /// 1. `[writable]` VaultConfig PDA
    /// 2. `[]` USDC Mint
    /// 3. `[writable]` Vault Token Account
    /// 4. `[]` System Program
    /// 5. `[]` Token Program
    /// 6. `[]` Rent Sysvar
    Initialize {
        delegation_program: Pubkey,
    },

    /// Index 1: 初始化用户账户
    ///
    /// Accounts:
    /// 0. `[signer]` User
    /// 1. `[writable]` UserAccount PDA (seeds: ["user", wallet, &[account_index]])
    /// 2. `[]` System Program
    InitializeUser {
        account_index: u8,
    },

    /// Index 2: 入金
    ///
    /// Accounts:
    /// 0. `[signer]` User
    /// 1. `[writable]` UserAccount PDA
    /// 2. `[writable]` User USDC Token Account
    /// 3. `[writable]` Vault USDC Token Account
    /// 4. `[writable]` VaultConfig
    /// 5. `[]` Token Program
    Deposit {
        amount: u64,
    },

    /// Index 3: 出金
    ///
    /// Accounts:
    /// 0. `[signer]` User
    /// 1. `[writable]` UserAccount PDA
    /// 2. `[writable]` User USDC Token Account
    /// 3. `[writable]` Vault USDC Token Account
    /// 4. `[]` VaultConfig
    /// 5. `[]` Token Program
    Withdraw {
        amount: u64,
    },

    /// Index 4: 添加授权调用方 (Governance Authority only)
    ///
    /// Accounts:
    /// 0. `[signer]` Governance Authority
    /// 1. `[writable]` VaultConfig
    AddAuthorizedCaller {
        caller: Pubkey,
    },

    /// Index 5: 移除授权调用方 (Governance Authority only)
    ///
    /// Accounts:
    /// 0. `[signer]` Governance Authority
    /// 1. `[writable]` VaultConfig
    RemoveAuthorizedCaller {
        caller: Pubkey,
    },

    /// Index 6: 暂停/恢复 (Governance Authority only)
    ///
    /// Accounts:
    /// 0. `[signer]` Governance Authority
    /// 1. `[writable]` VaultConfig
    SetPaused {
        paused: bool,
    },

    /// Index 7: 更新治理权限 (Governance Authority only)
    ///
    /// Accounts:
    /// 0. `[signer]` Current Governance Authority
    /// 1. `[writable]` VaultConfig
    UpdateGovernanceAuthority {
        new_governance_authority: Pubkey,
    },

    /// Index 8: Relayer 代理入金 (Governance Authority/Relayer only)
    ///
    /// PDA seeds: ["user", user_wallet, &[account_index]]
    ///
    /// Accounts:
    /// 0. `[signer]` Governance Authority/Relayer
    /// 1. `[writable]` UserAccount PDA (会自动创建)
    /// 2. `[writable]` VaultConfig
    /// 3. `[]` System Program (用于创建账户)
    RelayerDeposit {
        user_wallet: Pubkey,
        amount: u64,
        account_index: u8,
    },

    /// Index 9: Relayer 代理出金 (Governance Authority/Relayer only)
    ///
    /// PDA seeds: ["user", user_wallet, &[account_index]]
    ///
    /// Accounts:
    /// 0. `[signer]` Governance Authority/Relayer
    /// 1. `[writable]` UserAccount PDA
    /// 2. `[]` VaultConfig
    RelayerWithdraw {
        user_wallet: Pubkey,
        amount: u64,
        account_index: u8,
    },

    /// Index 10: Spot Token 入金 (用户直接调用)
    ///
    /// SPL Token 转入 Vault + 更新 SpotTokenBalance PDA (auto-init)
    ///
    /// Accounts:
    /// 0. `[signer]` User
    /// 1. `[writable]` SpotTokenBalance PDA (seeds: ["spot_balance", user, token_index])
    /// 2. `[writable]` User Token Account (SPL Token)
    /// 3. `[writable]` Vault Token Account (SPL Token)
    /// 4. `[]` VaultConfig
    /// 5. `[]` Token Program
    /// 6. `[]` System Program (for auto-init)
    SpotDeposit {
        token_index: u16,
        amount: u64,
        account_index: u8,
        amount_e6: i64,
    },

    /// Index 11: Spot Token 出金 (用户直接调用)
    ///
    /// Vault 中的 Token 转回给用户 + 更新 SpotTokenBalance PDA
    ///
    /// Accounts:
    /// 0. `[signer]` User
    /// 1. `[writable]` SpotTokenBalance PDA (seeds: ["spot_balance", user, token_index])
    /// 2. `[writable]` User Token Account (SPL Token)
    /// 3. `[writable]` Vault Token Account (SPL Token)
    /// 4. `[]` VaultConfig
    /// 5. `[]` Token Program
    SpotWithdraw {
        token_index: u16,
        amount: u64,
        account_index: u8,
        amount_e6: i64,
    },

    /// Index 12: Relayer 代理 Spot 入金 (Governance Authority/Relayer only)
    ///
    /// 更新 SpotTokenBalance PDA (auto-init if needed)
    ///
    /// Accounts:
    /// 0. `[signer]` Governance Authority/Relayer
    /// 1. `[writable]` SpotTokenBalance PDA (seeds: ["spot_balance", user_wallet, token_index])
    /// 2. `[]` VaultConfig
    /// 3. `[]` System Program (for auto-init)
    RelayerSpotDeposit {
        user_wallet: Pubkey,
        token_index: u16,
        amount: u64,
        account_index: u8,
        amount_e6: i64,
    },

    /// Index 13: Relayer 代理 Spot 出金 (Governance Authority/Relayer only)
    ///
    /// Accounts:
    /// 0. `[signer]` Governance Authority/Relayer
    /// 1. `[writable]` SpotTokenBalance PDA (seeds: ["spot_balance", user_wallet, token_index])
    /// 2. `[]` VaultConfig
    RelayerSpotWithdraw {
        user_wallet: Pubkey,
        token_index: u16,
        amount: u64,
        account_index: u8,
        amount_e6: i64,
    },

    /// Index 14: Relayer 代理出金并转账 (Governance Authority/Relayer only)
    ///
    /// 功能：
    /// 1. 扣除用户 Vault 余额（记账）
    /// 2. 从 Vault Token Account 转 USDC 到 Relayer Token Account
    ///
    /// 用途：跨链桥出金 — Relayer 从 Vault 提取 USDC 后调用 Bridge.stake 跨链
    ///
    /// Accounts:
    /// 0. `[signer]` Governance Authority/Relayer
    /// 1. `[writable]` UserAccount PDA
    /// 2. `[]` VaultConfig
    /// 3. `[writable]` Vault Token Account
    /// 4. `[writable]` Relayer Token Account (接收方)
    /// 5. `[]` Token Program
    RelayerWithdrawAndTransfer {
        user_wallet: Pubkey,
        amount: u64,
        account_index: u8,
    },

    /// Index 15: UserAccount state (Relayer-only, set-to-value)
    ///
    /// Sets UserAccount PDA fields to exact values (idempotent, not add/subtract).
    ///
    /// Accounts:
    /// 0. `[signer]` Governance Authority/Relayer
    /// 1. `[writable]` UserAccount PDA
    /// 2. `[]` VaultConfig
    /// 3. `[]` System Program (for auto-init if PDA doesn't exist)
    UserAccount {
        user_wallet: Pubkey,
        account_index: u8,
        available_balance_e6: i64,
        locked_margin_e6: i64,
        spot_locked_e6: i64,
        oracle_locked_e6: i64,
    },

    /// Index 16: SpotTokenBalance state (Relayer-only, set-to-value)
    ///
    /// Sets SpotTokenBalance PDA fields to exact values (idempotent).
    ///
    /// Accounts:
    /// 0. `[signer]` Governance Authority/Relayer
    /// 1. `[writable]` SpotTokenBalance PDA
    /// 2. `[]` VaultConfig
    /// 3. `[]` System Program (for auto-init if PDA doesn't exist)
    SpotTokenBalance {
        user_wallet: Pubkey,
        account_index: u8,
        token_index: u16,
        available_e6: i64,
        locked_e6: i64,
    },

    /// Index 17: Migrate VaultConfig from V1 (569 bytes) to V2 (505 bytes)
    ///
    /// Removes deprecated ledger_program and fund_program fields.
    ///
    /// Accounts:
    /// 0. `[signer]` Governance Authority
    /// 1. `[writable]` VaultConfig PDA
    /// 2. `[]` System Program
    MigrateVaultConfig,
}
