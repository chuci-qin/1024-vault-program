//! Vault Program Error Types

use solana_program::program_error::ProgramError;
use thiserror::Error;

#[derive(Error, Debug, Copy, Clone)]
pub enum VaultError {
    #[error("Insufficient balance")]
    InsufficientBalance,

    #[error("Vault is paused")]
    VaultPaused,

    #[error("Invalid amount")]
    InvalidAmount,

    #[error("Invalid account")]
    InvalidAccount,

    #[error("Numerical overflow")]
    Overflow,

    #[error("Invalid PDA")]
    InvalidPda,

    #[error("Account already initialized")]
    AlreadyInitialized,

    #[error("Account not initialized")]
    NotInitialized,

    #[error("Invalid governance authority")]
    InvalidGovernanceAuthority,

    #[error("Invalid relayer")]
    InvalidRelayer,

    #[error("Unauthorized governance authority")]
    UnauthorizedGovernanceAuthority,

    #[error("Unauthorized user")]
    UnauthorizedUser,

    #[error("Quote asset must use Vault Deposit/Withdraw path")]
    QuoteAssetMustUseVaultPath,
}

impl From<VaultError> for ProgramError {
    fn from(e: VaultError) -> Self {
        ProgramError::Custom(e as u32)
    }
}
