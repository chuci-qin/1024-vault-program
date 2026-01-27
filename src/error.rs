//! Vault Program Error Types

use solana_program::program_error::ProgramError;
use thiserror::Error;

#[derive(Error, Debug, Copy, Clone)]
pub enum VaultError {
    /// Invalid instruction
    #[error("Invalid instruction")]
    InvalidInstruction,

    /// Insufficient balance
    #[error("Insufficient balance")]
    InsufficientBalance,

    /// Insufficient margin
    #[error("Insufficient margin")]
    InsufficientMargin,

    /// Unauthorized caller (not in whitelist)
    #[error("Unauthorized caller")]
    UnauthorizedCaller,

    /// Vault is paused
    #[error("Vault is paused")]
    VaultPaused,

    /// Invalid amount (must be > 0)
    #[error("Invalid amount")]
    InvalidAmount,

    /// Invalid account
    #[error("Invalid account")]
    InvalidAccount,

    /// Numerical overflow
    #[error("Numerical overflow")]
    Overflow,

    /// Insurance fund insufficient
    #[error("Insurance fund insufficient")]
    InsuranceFundInsufficient,

    /// Invalid PDA
    #[error("Invalid PDA")]
    InvalidPda,

    /// Account already initialized
    #[error("Account already initialized")]
    AlreadyInitialized,

    /// Account not initialized
    #[error("Account not initialized")]
    NotInitialized,

    /// Invalid admin
    #[error("Invalid admin")]
    InvalidAdmin,

    /// Invalid CPI caller PDA (P0-1 fix: CPI caller must be a valid PDA)
    #[error("Invalid CPI caller PDA")]
    InvalidCallerPda,

    /// CPI caller is not a signer
    #[error("CPI caller must be a signer")]
    CallerNotSigner,

    /// Invalid relayer (not admin or authorized relayer)
    #[error("Invalid relayer")]
    InvalidRelayer,

    // Spot 相关错误

    /// Unauthorized admin
    #[error("Unauthorized admin")]
    UnauthorizedAdmin,

    /// Unauthorized user
    #[error("Unauthorized user")]
    UnauthorizedUser,

    /// Deposit failed
    #[error("Deposit failed")]
    DepositFailed,

    /// Settlement failed
    #[error("Settlement failed")]
    SettlementFailed,

    /// Token slots full (max 64)
    #[error("Token slots full")]
    TokenSlotsFull,

    /// Missing required signature
    #[error("Missing required signature")]
    MissingSignature,

    /// Invalid user account PDA
    #[error("Invalid user account PDA")]
    InvalidUserAccount,

    /// Recurring auth is not active
    #[error("Recurring auth is not active")]
    RecurringAuthNotActive,

    /// Invalid cycle count
    #[error("Invalid cycle count")]
    InvalidCycleCount,

    /// Recurring auth execution failed
    #[error("Recurring auth execution failed")]
    RecurringAuthExecutionFailed,
}

impl From<VaultError> for ProgramError {
    fn from(e: VaultError) -> Self {
        ProgramError::Custom(e as u32)
    }
}

