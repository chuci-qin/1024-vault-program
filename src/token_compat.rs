//! Token Program Compatibility Layer
//! 
//! This module provides compatibility between SPL Token (v1) and SPL Token-2022 (v2).
//! Both programs use the same instruction format for basic operations, so we can
//! use the spl_token crate with different program IDs.
//!
//! Key insight: Token-2022 uses the same instruction encoding as Token v1 for
//! basic operations (transfer, mint_to, burn, initialize_account3, initialize_mint2).

use solana_program::{
    account_info::AccountInfo,
    entrypoint::ProgramResult,
    program::{invoke, invoke_signed},
    program_pack::Pack,
    pubkey::Pubkey,
};

/// SPL Token Program ID (v1)
pub const TOKEN_PROGRAM_V1: Pubkey = spl_token::id();

/// SPL Token-2022 Program ID (v2)
/// TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb
pub const TOKEN_PROGRAM_V2: Pubkey = solana_program::pubkey!("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb");

/// Check if a program ID is a valid token program (v1 or v2)
pub fn is_valid_token_program(program_id: &Pubkey) -> bool {
    *program_id == TOKEN_PROGRAM_V1 || *program_id == TOKEN_PROGRAM_V2
}

/// Get the token account size (same for both v1 and v2 for basic accounts)
pub fn get_token_account_size(_token_program_id: &Pubkey) -> usize {
    // Both v1 and v2 use 165 bytes for basic token accounts
    spl_token::state::Account::LEN
}

/// Get the mint size (same for both v1 and v2 for basic mints)
#[allow(dead_code)]
pub fn get_mint_size(_token_program_id: &Pubkey) -> usize {
    // Both v1 and v2 use 82 bytes for basic mints
    spl_token::state::Mint::LEN
}

/// Create a TransferChecked instruction (works for both v1 and v2).
/// Preferred over Transfer for Token-2022 compatibility — validates decimals
/// and mint, preventing silent truncation or wrong-mint transfers.
pub fn create_transfer_checked_instruction(
    token_program_id: &Pubkey,
    source: &Pubkey,
    mint: &Pubkey,
    destination: &Pubkey,
    authority: &Pubkey,
    amount: u64,
    decimals: u8,
) -> Result<solana_program::instruction::Instruction, solana_program::program_error::ProgramError> {
    // TransferChecked is instruction 12 in both v1 and v2
    // Format: [instruction_type (1 byte)] + [amount (8 bytes LE)] + [decimals (1 byte)]
    let mut data = Vec::with_capacity(10);
    data.push(12u8); // TransferChecked instruction
    data.extend_from_slice(&amount.to_le_bytes());
    data.push(decimals);

    Ok(solana_program::instruction::Instruction {
        program_id: *token_program_id,
        accounts: vec![
            solana_program::instruction::AccountMeta::new(*source, false),
            solana_program::instruction::AccountMeta::new_readonly(*mint, false),
            solana_program::instruction::AccountMeta::new(*destination, false),
            solana_program::instruction::AccountMeta::new_readonly(*authority, true),
        ],
        data,
    })
}

/// TransferChecked with dynamic program support.
/// Preferred for Token-2022 tokens; requires the mint account to validate decimals.
pub fn transfer_checked<'a>(
    token_program: &AccountInfo<'a>,
    source: &AccountInfo<'a>,
    mint: &AccountInfo<'a>,
    destination: &AccountInfo<'a>,
    authority: &AccountInfo<'a>,
    amount: u64,
    decimals: u8,
    signer_seeds: Option<&[&[u8]]>,
) -> ProgramResult {
    let ix = create_transfer_checked_instruction(
        token_program.key,
        source.key,
        mint.key,
        destination.key,
        authority.key,
        amount,
        decimals,
    )?;

    let account_infos = vec![
        source.clone(),
        mint.clone(),
        destination.clone(),
        authority.clone(),
    ];

    if let Some(seeds) = signer_seeds {
        invoke_signed(&ix, &account_infos, &[seeds])
    } else {
        invoke(&ix, &account_infos)
    }
}

/// Create a Transfer instruction (works for both v1 and v2)
/// CHAIN-3 NOTE: For Token-2022 compatibility, prefer `transfer_checked` which
/// validates decimals and mint.  This `transfer` function is retained for
/// backward compatibility with existing Token v1 call sites (Deposit/Withdraw).
pub fn create_transfer_instruction(
    token_program_id: &Pubkey,
    source: &Pubkey,
    destination: &Pubkey,
    authority: &Pubkey,
    amount: u64,
) -> Result<solana_program::instruction::Instruction, solana_program::program_error::ProgramError> {
    // Transfer is instruction 3 in both v1 and v2
    // Format: [instruction_type (1 byte)] + [amount (8 bytes LE)]
    let mut data = Vec::with_capacity(9);
    data.push(3u8); // Transfer instruction
    data.extend_from_slice(&amount.to_le_bytes());
    
    Ok(solana_program::instruction::Instruction {
        program_id: *token_program_id,
        accounts: vec![
            solana_program::instruction::AccountMeta::new(*source, false),
            solana_program::instruction::AccountMeta::new(*destination, false),
            solana_program::instruction::AccountMeta::new_readonly(*authority, true),
        ],
        data,
    })
}

/// Transfer tokens with dynamic program support.
/// CHAIN-3: For new Spot token integrations (especially Token-2022), use
/// `transfer_checked` instead.  Current Deposit/Withdraw paths use USDC
/// (Token v1) where Transfer is safe, but SpotDeposit/SpotWithdraw should
/// migrate to `transfer_checked` when adding Token-2022 asset support.
pub fn transfer<'a>(
    token_program: &AccountInfo<'a>,
    source: &AccountInfo<'a>,
    destination: &AccountInfo<'a>,
    authority: &AccountInfo<'a>,
    amount: u64,
    signer_seeds: Option<&[&[u8]]>,
) -> ProgramResult {
    let ix = create_transfer_instruction(
        token_program.key,
        source.key,
        destination.key,
        authority.key,
        amount,
    )?;

    let account_infos = vec![
        source.clone(),
        destination.clone(),
        authority.clone(),
    ];

    if let Some(seeds) = signer_seeds {
        invoke_signed(&ix, &account_infos, &[seeds])
    } else {
        invoke(&ix, &account_infos)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_valid_token_program() {
        assert!(is_valid_token_program(&TOKEN_PROGRAM_V1));
        assert!(is_valid_token_program(&TOKEN_PROGRAM_V2));
        assert!(!is_valid_token_program(&Pubkey::default()));
    }

    #[test]
    fn test_get_token_account_size() {
        assert_eq!(get_token_account_size(&TOKEN_PROGRAM_V1), 165);
        assert_eq!(get_token_account_size(&TOKEN_PROGRAM_V2), 165);
    }

    #[test]
    fn test_get_mint_size() {
        assert_eq!(get_mint_size(&TOKEN_PROGRAM_V1), 82);
        assert_eq!(get_mint_size(&TOKEN_PROGRAM_V2), 82);
    }
}
