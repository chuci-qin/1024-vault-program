//! 1024 DEX Vault Program
//! 
//! 完全去中心化的资金托管程序
//! - 用户资金100%由链上程序托管
//! - 入金/出金操作
//! - 保证金锁定/释放
//! - 保险基金管理
//! - 清算结算

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
pub mod cpi;

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

