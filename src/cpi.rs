//! Vault Program CPI Helper Functions
//! 
//! These functions are used by Exchange Program for PM Oracle bond operations.
//! 
//! Deprecated CPI functions (lock_margin, release_margin, close_position_settle)
//! have been removed — all settlement is now DB-First via balance_ops.

use crate::instruction::VaultInstruction;
use borsh::BorshSerialize;
use solana_program::{
    account_info::AccountInfo,
    entrypoint::ProgramResult,
    instruction::{AccountMeta, Instruction},
    program::invoke_signed,
    pubkey::Pubkey,
};

/// Lock bond for PM Oracle proposal (CPI)
pub fn lock_bond<'a>(
    vault_program_id: &Pubkey,
    vault_config: AccountInfo<'a>,
    user_account: AccountInfo<'a>,
    caller_program: AccountInfo<'a>,
    amount_e6: u64,
    signers_seeds: &[&[&[u8]]],
) -> ProgramResult {
    let instruction = Instruction {
        program_id: *vault_program_id,
        accounts: vec![
            AccountMeta::new_readonly(*vault_config.key, false),
            AccountMeta::new(*user_account.key, false),
            AccountMeta::new_readonly(*caller_program.key, false),
        ],
        data: VaultInstruction::LockBond { amount_e6 }.try_to_vec()?,
    };

    invoke_signed(
        &instruction,
        &[vault_config, user_account, caller_program],
        signers_seeds,
    )
}

/// Release bond from PM Oracle (CPI)
pub fn release_bond<'a>(
    vault_program_id: &Pubkey,
    vault_config: AccountInfo<'a>,
    user_account: AccountInfo<'a>,
    caller_program: AccountInfo<'a>,
    amount_e6: u64,
    signers_seeds: &[&[&[u8]]],
) -> ProgramResult {
    let instruction = Instruction {
        program_id: *vault_program_id,
        accounts: vec![
            AccountMeta::new_readonly(*vault_config.key, false),
            AccountMeta::new(*user_account.key, false),
            AccountMeta::new_readonly(*caller_program.key, false),
        ],
        data: VaultInstruction::ReleaseBond { amount_e6 }.try_to_vec()?,
    };

    invoke_signed(
        &instruction,
        &[vault_config, user_account, caller_program],
        signers_seeds,
    )
}

