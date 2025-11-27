//! Vault Program CPI Helper Functions
//! 
//! 这些函数供其他程序（Ledger, Fund）通过CPI调用Vault Program
//!
//! 架构说明:
//! - Vault Program 仅处理用户资金 (入金/出金/保证金)
//! - 清算罚金/穿仓覆盖等由 Ledger Program 调用 Fund Program 处理

use crate::instruction::VaultInstruction;
use borsh::BorshSerialize;
use solana_program::{
    account_info::AccountInfo,
    entrypoint::ProgramResult,
    instruction::{AccountMeta, Instruction},
    program::invoke_signed,
    pubkey::Pubkey,
};

/// 锁定保证金 (CPI)
/// 
/// # Arguments
/// * `vault_program_id` - Vault Program ID
/// * `vault_config` - VaultConfig账户
/// * `user_account` - UserAccount PDA
/// * `caller_program` - 调用方程序账户
/// * `amount` - 锁定金额 (e6)
/// * `signers_seeds` - PDA签名种子
pub fn lock_margin<'a>(
    vault_program_id: &Pubkey,
    vault_config: AccountInfo<'a>,
    user_account: AccountInfo<'a>,
    caller_program: AccountInfo<'a>,
    amount: u64,
    signers_seeds: &[&[&[u8]]],
) -> ProgramResult {
    let instruction = Instruction {
        program_id: *vault_program_id,
        accounts: vec![
            AccountMeta::new_readonly(*vault_config.key, false),
            AccountMeta::new(*user_account.key, false),
            AccountMeta::new_readonly(*caller_program.key, false),
        ],
        data: VaultInstruction::LockMargin { amount }.try_to_vec()?,
    };

    invoke_signed(
        &instruction,
        &[vault_config, user_account, caller_program],
        signers_seeds,
    )
}

/// 释放保证金 (CPI)
pub fn release_margin<'a>(
    vault_program_id: &Pubkey,
    vault_config: AccountInfo<'a>,
    user_account: AccountInfo<'a>,
    caller_program: AccountInfo<'a>,
    amount: u64,
    signers_seeds: &[&[&[u8]]],
) -> ProgramResult {
    let instruction = Instruction {
        program_id: *vault_program_id,
        accounts: vec![
            AccountMeta::new_readonly(*vault_config.key, false),
            AccountMeta::new(*user_account.key, false),
            AccountMeta::new_readonly(*caller_program.key, false),
        ],
        data: VaultInstruction::ReleaseMargin { amount }.try_to_vec()?,
    };

    invoke_signed(
        &instruction,
        &[vault_config, user_account, caller_program],
        signers_seeds,
    )
}

/// 平仓结算 (CPI - 仅用户账户部分)
/// 
/// 注意: 手续费分配由 Ledger Program 单独调用 Fund Program 处理
/// 
/// # Arguments
/// * `margin_to_release` - 释放的保证金 (e6)
/// * `realized_pnl` - 实现盈亏 (e6, 正=盈利, 负=亏损)
/// * `fee` - 手续费 (e6) - 从用户余额扣除
pub fn close_position_settle<'a>(
    vault_program_id: &Pubkey,
    vault_config: AccountInfo<'a>,
    user_account: AccountInfo<'a>,
    caller_program: AccountInfo<'a>,
    margin_to_release: u64,
    realized_pnl: i64,
    fee: u64,
    signers_seeds: &[&[&[u8]]],
) -> ProgramResult {
    let instruction = Instruction {
        program_id: *vault_program_id,
        accounts: vec![
            AccountMeta::new_readonly(*vault_config.key, false),
            AccountMeta::new(*user_account.key, false),
            AccountMeta::new_readonly(*caller_program.key, false),
        ],
        data: VaultInstruction::ClosePositionSettle {
            margin_to_release,
            realized_pnl,
            fee,
        }
        .try_to_vec()?,
    };

    invoke_signed(
        &instruction,
        &[vault_config, user_account, caller_program],
        signers_seeds,
    )
}

/// 清算用户账户 (CPI)
/// 
/// 执行完整的清算资金处理:
/// 1. 更新用户账户状态 (清空保证金, 返还剩余)
/// 2. 将清算罚金从 Vault Token Account 转入 Insurance Fund Vault
/// 
/// # Arguments
/// * `margin` - 用户锁定的保证金 (e6) - 将被清空
/// * `user_remainder` - 返还给用户的剩余 (e6)
/// * `liquidation_penalty` - 清算罚金 (e6) - 转入 Insurance Fund
/// * `vault_token_account` - Vault 的 Token 账户 (源)
/// * `insurance_fund_vault` - Insurance Fund 的 Token 账户 (目标)
/// * `token_program` - SPL Token Program
pub fn liquidate_position<'a>(
    vault_program_id: &Pubkey,
    vault_config: AccountInfo<'a>,
    user_account: AccountInfo<'a>,
    caller_program: AccountInfo<'a>,
    vault_token_account: AccountInfo<'a>,
    insurance_fund_vault: AccountInfo<'a>,
    token_program: AccountInfo<'a>,
    margin: u64,
    user_remainder: u64,
    liquidation_penalty: u64,
    signers_seeds: &[&[&[u8]]],
) -> ProgramResult {
    let instruction = Instruction {
        program_id: *vault_program_id,
        accounts: vec![
            AccountMeta::new_readonly(*vault_config.key, false),
            AccountMeta::new(*user_account.key, false),
            AccountMeta::new_readonly(*caller_program.key, false),
            AccountMeta::new(*vault_token_account.key, false),
            AccountMeta::new(*insurance_fund_vault.key, false),
            AccountMeta::new_readonly(*token_program.key, false),
        ],
        data: VaultInstruction::LiquidatePosition {
            margin,
            user_remainder,
            liquidation_penalty,
        }
        .try_to_vec()?,
    };

    invoke_signed(
        &instruction,
        &[
            vault_config,
            user_account,
            caller_program,
            vault_token_account,
            insurance_fund_vault,
            token_program,
        ],
        signers_seeds,
    )
}
