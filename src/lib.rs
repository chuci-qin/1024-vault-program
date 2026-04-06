//! 1024 DEX Vault Program
//! 
//! User fund custody program (DB-First architecture).
//! 18 active instructions for deposit/withdraw, Spot token management,
//! on-chain state mirrors (UserAccount, SpotTokenBalance), and governance.

use solana_program::{
    account_info::AccountInfo,
    entrypoint,
    entrypoint::ProgramResult,
    pubkey::Pubkey,
};

pub mod error;
pub mod instruction;
pub mod processor;
pub mod state;
pub mod token_compat;
pub mod utils;

#[cfg(not(feature = "no-entrypoint"))]
entrypoint!(process_instruction);

/// Program entrypoint
pub fn process_instruction(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    instruction_data: &[u8],
) -> ProgramResult {
    processor::process_instruction(program_id, accounts, instruction_data)
}

