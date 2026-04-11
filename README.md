# 1024 Vault Program

User fund custody program for the 1024 DEX platform.

## Architecture

**DB-First + On-chain Real-time Audit.** The Vault Program is one of only two active on-chain programs:

- **Vault Program** — Fund custody (deposit, withdraw, balance state)
- **Exchange Program** — Immutable audit trail (trades, positions, liquidations)

All trading logic executes in the backend database. The Vault Program handles:

1. **Real SPL Token transfers** — User deposit/withdraw, Spot deposit/withdraw, Relayer-assisted cross-chain
2. **On-chain state** — UserAccount and SpotTokenBalance PDAs reflect DB state (idempotent, set-to-value)
3. **Governance operations** — Pause/resume, authorized caller management, authority transfer

## Instructions (18 variants)

| Index | Instruction | Signer | Description |
|:-----:|-------------|--------|-------------|
| 0 | `Initialize` | Governance Authority | Create VaultConfig PDA |
| 1 | `InitializeUser` | User | Create UserAccount PDA |
| 2 | `Deposit` | User | USDC deposit (SPL Token transfer into Vault) |
| 3 | `Withdraw` | User | USDC withdrawal (SPL Token transfer from Vault) |
| 4 | `AddAuthorizedCaller` | Governance Authority | Add a program to the authorized callers list |
| 5 | `RemoveAuthorizedCaller` | Governance Authority | Remove a program from the authorized callers list |
| 6 | `SetPaused` | Governance Authority | Pause/resume the Vault |
| 7 | `UpdateGovernanceAuthority` | Governance Authority | Transfer governance authority to a new key |
| 8 | `RelayerDeposit` | Relayer | Relayer-assisted USDC deposit (auto-init UserAccount) |
| 9 | `RelayerWithdraw` | Relayer | Relayer-assisted USDC withdrawal |
| 10 | `SpotDeposit` | User | SPL Token deposit (wBTC/wETH/wSOL) into Vault |
| 11 | `SpotWithdraw` | User | SPL Token withdrawal from Vault |
| 12 | `RelayerSpotDeposit` | Relayer | Relayer-assisted Spot deposit (auto-init PDA) |
| 13 | `RelayerSpotWithdraw` | Relayer | Relayer-assisted Spot withdrawal |
| 14 | `RelayerWithdrawAndTransfer` | Relayer | Cross-chain bridge: debit UserAccount + transfer USDC to Relayer |
| 15 | `UserAccount` | Relayer | Set UserAccount PDA to DB state (idempotent) |
| 16 | `SpotTokenBalance` | Relayer | Set SpotTokenBalance PDA to DB state (idempotent) |
| 17 | `MigrateVaultConfig` | Governance Authority | One-time V1 (569 bytes) to V2 (505 bytes) migration |

## PDA Seeds

| Account | Seeds | Size |
|---------|-------|:----:|
| VaultConfig | `["vault_config"]` | 505 bytes |
| UserAccount | `["user", wallet, &[account_index]]` | 153 bytes |
| SpotTokenBalance | `["spot_balance", wallet, &[account_index], token_index.to_le_bytes()]` | 98 bytes |

## State Structs

### VaultConfig (505 bytes)

Global program configuration. Stores governance authority, USDC mint, vault token account, and up to 10 authorized callers.

### UserAccount (153 bytes)

Per-user per-sub-account balance state. Fields: `available_balance_e6`, `locked_margin_e6`, `spot_locked_e6`, `oracle_locked_e6`, `unrealized_pnl_e6`, etc. `account_index=0` is the main account; 1-255 are sub-accounts.

### SpotTokenBalance (98 bytes)

Per-token balance PDA. Each (wallet, account_index, token_index) triple gets its own PDA, auto-created on first use. Fields: `available_e6`, `locked_e6`.

## Error Codes

| Code | Name | Description |
|:----:|------|-------------|
| 0 | `InsufficientBalance` | Balance too low for operation |
| 1 | `VaultPaused` | Vault is currently paused |
| 2 | `InvalidAmount` | Amount is zero or invalid |
| 3 | `InvalidAccount` | Account does not match expected |
| 4 | `Overflow` | Numerical overflow |
| 5 | `InvalidPda` | PDA derivation mismatch |
| 6 | `AlreadyInitialized` | Account already initialized |
| 7 | `NotInitialized` | Account not yet initialized |
| 8 | `InvalidGovernanceAuthority` | Signer is not the governance authority |
| 9 | `InvalidRelayer` | Signer is not an authorized relayer |
| 10 | `UnauthorizedGovernanceAuthority` | Governance authority check failed |
| 11 | `UnauthorizedUser` | User authorization check failed |
| 12 | `QuoteAssetMustUseVaultPath` | USDC must use Deposit/Withdraw, not SpotDeposit/SpotWithdraw |

## Source Files

```
src/
  lib.rs           — Entrypoint
  instruction.rs   — VaultInstruction enum (18 variants)
  processor.rs     — Instruction dispatch and handlers
  state.rs         — VaultConfig, UserAccount, SpotTokenBalance
  error.rs         — VaultError enum (13 variants)
  utils.rs         — Signer/writable assertions, checked arithmetic
  token_compat.rs  — SPL Token transfer helpers
```

## Build

```bash
cargo build-sbf
# Output: target/deploy/vault_program.so
```

## Program IDs

| Environment | Program ID |
|-------------|-----------|
| Local Testnet | `EKsHPHtZmHRH9TFNGPVFp7MWNFuBcYZj1mdv87F9aSNt` |
| Testnet-Stable | `BxMAToJxZYZ2iTrFL4cRAVL9pHZyakMvjbk1LTLHi9Nh` |
| Mainnet | `C3pDwbciRtrxDr2Qfuqw67EUb9DHBJsAnmhty1jfk9fF` |
