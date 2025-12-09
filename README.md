# 1024 Vault Program

> 完全去中心化的资金托管程序 - 用户资金安全的核心保障

---

## 📋 目录

- [概述](#概述)
- [架构设计](#架构设计)
- [账户结构](#账户结构)
- [指令详解](#指令详解)
- [PDA 地址推导](#pda-地址推导)
- [CPI 集成指南](#cpi-集成指南)
- [安全机制](#安全机制)
- [构建与部署](#构建与部署)
- [测试](#测试)
- [错误代码](#错误代码)

---

## 概述

### 程序职责

1024 Vault Program 是 1024 DEX 生态系统的资金托管核心，负责：

| 职责 | 说明 |
|------|------|
| **资金托管** | 100% 链上托管，平台绝不接触用户私钥 |
| **入金/出金** | 用户自主的 USDC 存取操作 |
| **保证金管理** | 锁定/释放交易保证金 |
| **清算结算** | 与 Ledger Program 配合的仓位清算 |
| **预测市场资金** | 独立的预测市场用户账户 |
| **跨链入金 (Relayer)** | 支持任意链资产无缝入金 |

### 部署信息

| 网络 | Program ID |
|------|-----------|
| 1024Chain Testnet | `vR3BifKCa2TGKP2uhToxZAMYAYydqpesvKGX54gzFny` |
| 1024Chain Mainnet | TBD |

### 依赖关系

```
                    ┌─────────────────────┐
                    │   1024-vault-program │
                    │   (资金托管)          │
                    └─────────┬───────────┘
                              │
        ┌─────────────────────┼─────────────────────┐
        │                     │                     │
        ▼                     ▼                     ▼
┌───────────────┐   ┌─────────────────┐   ┌─────────────────┐
│ Ledger Program│   │  Fund Program   │   │ Prediction      │
│ (CPI 调用)     │   │ (保险基金)       │   │ Market Program  │
└───────────────┘   └─────────────────┘   └─────────────────┘
```

---

## 架构设计

### 设计原则

```
Vault Program = 用户资金托管 (用户的钱)
Fund Program  = 资金池管理 (保险基金/手续费/返佣)
```

### 资金流向

```
┌─────────────────────────────────────────────────────────────────┐
│                      资金流向示意图                               │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│   用户钱包 (Solana/EVM)                                          │
│       │                                                         │
│       │  Deposit / RelayerDeposit                               │
│       ▼                                                         │
│   ┌─────────────────────────────────────┐                       │
│   │         Vault Token Account          │                       │
│   │   (存放所有用户的 USDC)               │                       │
│   └─────────────────────┬───────────────┘                       │
│                         │                                        │
│         ┌───────────────┼───────────────┐                       │
│         │               │               │                       │
│         ▼               ▼               ▼                       │
│   ┌───────────┐   ┌───────────┐   ┌───────────────────┐        │
│   │UserAccount│   │UserAccount│   │PredictionMarket   │        │
│   │  User A   │   │  User B   │   │UserAccount        │        │
│   │           │   │           │   │                   │        │
│   │ balance   │   │ balance   │   │ pm_locked         │        │
│   │ locked    │   │ locked    │   │ pm_pending        │        │
│   └───────────┘   └───────────┘   └───────────────────┘        │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘
```

---

## 账户结构

### 1. VaultConfig (全局配置)

**PDA Seeds:** `["vault_config"]`

**大小:** ~400 bytes

```rust
pub struct VaultConfig {
    pub discriminator: u64,              // 账户类型标识 "VAULT_CO"
    pub admin: Pubkey,                   // 管理员
    pub usdc_mint: Pubkey,               // USDC Mint 地址
    pub vault_token_account: Pubkey,     // Vault USDC 存储账户
    pub authorized_callers: Vec<Pubkey>, // CPI 授权白名单 (最多10个)
    pub ledger_program: Pubkey,          // Ledger Program ID
    pub fund_program: Option<Pubkey>,    // Fund Program ID
    pub delegation_program: Pubkey,      // Delegation Program ID
    pub total_deposits: u64,             // 总存款 (e6)
    pub total_locked: u64,               // 总锁定保证金 (e6)
    pub is_paused: bool,                 // 暂停状态
}
```

### 2. UserAccount (用户账户)

**PDA Seeds:** `["user", wallet_pubkey]`

**大小:** 153 bytes

```rust
pub struct UserAccount {
    pub discriminator: u64,          // 账户类型标识 "USER_ACC"
    pub wallet: Pubkey,              // 用户钱包地址
    pub bump: u8,                    // PDA bump
    pub available_balance_e6: i64,   // 可用余额 (e6)
    pub locked_margin_e6: i64,       // 锁定保证金 (e6)
    pub unrealized_pnl_e6: i64,      // 未实现盈亏 (e6)
    pub total_deposited_e6: i64,     // 累计存款 (e6)
    pub total_withdrawn_e6: i64,     // 累计提款 (e6)
    pub last_update_ts: i64,         // 最后更新时间
    pub reserved: [u8; 64],          // 预留扩展
}
```

**关键方法:**

```rust
impl UserAccount {
    /// 计算用户权益 (可用 + 锁定 + 未实现盈亏)
    pub fn equity(&self) -> i64 {
        self.available_balance_e6 + self.locked_margin_e6 + self.unrealized_pnl_e6
    }
}
```

### 3. PredictionMarketUserAccount (预测市场用户账户)

**PDA Seeds:** `["prediction_market_user", wallet_pubkey]`

**大小:** 161 bytes

```rust
pub struct PredictionMarketUserAccount {
    pub discriminator: u64,                      // "PM_USER"
    pub wallet: Pubkey,                          // 用户钱包
    pub bump: u8,                                // PDA bump
    pub prediction_market_locked_e6: i64,        // 预测市场锁定资金
    pub prediction_market_pending_settlement_e6: i64, // 待结算收益
    pub prediction_market_total_deposited_e6: i64,    // 累计存入
    pub prediction_market_total_withdrawn_e6: i64,    // 累计提取
    pub prediction_market_realized_pnl_e6: i64,       // 已实现盈亏
    pub last_update_ts: i64,                     // 最后更新
    pub reserved: [u8; 64],                      // 预留
}
```

---

## 指令详解

### 用户操作指令

#### 1. InitializeUser

初始化用户账户 PDA。

```rust
InitializeUser
```

| 账户 | 类型 | 说明 |
|------|------|------|
| 0 | `[signer]` | 用户钱包 |
| 1 | `[writable]` | UserAccount PDA |
| 2 | `[]` | System Program |

**示例 (TypeScript):**

```typescript
const [userAccountPDA] = await PublicKey.findProgramAddress(
    [Buffer.from("user"), wallet.publicKey.toBuffer()],
    VAULT_PROGRAM_ID
);

const ix = new TransactionInstruction({
    keys: [
        { pubkey: wallet.publicKey, isSigner: true, isWritable: false },
        { pubkey: userAccountPDA, isSigner: false, isWritable: true },
        { pubkey: SystemProgram.programId, isSigner: false, isWritable: false },
    ],
    programId: VAULT_PROGRAM_ID,
    data: Buffer.from([1]), // InitializeUser instruction index
});
```

#### 2. Deposit

用户存入 USDC。

```rust
Deposit { amount: u64 }
```

| 账户 | 类型 | 说明 |
|------|------|------|
| 0 | `[signer]` | 用户钱包 |
| 1 | `[writable]` | UserAccount PDA |
| 2 | `[writable]` | 用户 USDC Token Account |
| 3 | `[writable]` | Vault USDC Token Account |
| 4 | `[writable]` | VaultConfig |
| 5 | `[]` | Token Program |

#### 3. Withdraw

用户提取 USDC。

```rust
Withdraw { amount: u64 }
```

| 账户 | 类型 | 说明 |
|------|------|------|
| 0 | `[signer]` | 用户钱包 |
| 1 | `[writable]` | UserAccount PDA |
| 2 | `[writable]` | 用户 USDC Token Account |
| 3 | `[writable]` | Vault USDC Token Account |
| 4 | `[]` | VaultConfig |
| 5 | `[]` | Token Program |

### CPI 操作指令

> ⚠️ 这些指令只能由白名单中的 Program 通过 CPI 调用

#### 4. LockMargin

锁定用户保证金（开仓时）。

```rust
LockMargin { amount: u64 }
```

| 账户 | 类型 | 说明 |
|------|------|------|
| 0 | `[]` | VaultConfig |
| 1 | `[writable]` | UserAccount |
| 2 | `[]` | Caller Program (验证白名单) |

#### 5. ReleaseMargin

释放用户保证金（平仓时）。

```rust
ReleaseMargin { amount: u64 }
```

#### 6. ClosePositionSettle

平仓结算（合并操作）。

```rust
ClosePositionSettle {
    margin_to_release: u64,  // 释放的保证金
    realized_pnl: i64,       // 实现盈亏 (+/-) 
    fee: u64,                // 手续费
}
```

#### 7. LiquidatePosition

清算仓位。

```rust
LiquidatePosition {
    margin: u64,             // 用户锁定的保证金
    user_remainder: u64,     // 返还给用户的剩余
    liquidation_penalty: u64, // 清算罚金 → Insurance Fund
}
```

### 预测市场指令

#### 8. InitializePredictionMarketUser

创建预测市场用户账户。

#### 9. PredictionMarketLock

从 UserAccount 锁定资金到预测市场。

```rust
PredictionMarketLock { amount: u64 }
```

#### 10. PredictionMarketUnlock

从预测市场释放资金回 UserAccount。

#### 11. PredictionMarketSettle

预测市场结算。

```rust
PredictionMarketSettle {
    locked_amount: u64,      // 原锁定金额
    settlement_amount: u64,  // 结算应得金额
}
```

#### 12. PredictionMarketClaimSettlement

用户领取预测市场结算收益。

### Relayer 指令

> 用于跨链入金场景，用户无需在 1024Chain 上签名

#### 13. RelayerDeposit

Relayer 代理入金。

```rust
RelayerDeposit {
    user_wallet: Pubkey,  // 目标用户
    amount: u64,          // 入金金额
}
```

**特性:**
- 如果 UserAccount 不存在，自动创建
- 仅 Admin 可调用
- 不涉及实际 Token 转账（余额凭证模式）

#### 14. RelayerWithdraw

Relayer 代理出金。

```rust
RelayerWithdraw {
    user_wallet: Pubkey,
    amount: u64,
}
```

### 管理员指令

| 指令 | 说明 |
|------|------|
| `Initialize` | 初始化 Vault 配置 |
| `AddAuthorizedCaller` | 添加 CPI 白名单 |
| `RemoveAuthorizedCaller` | 移除 CPI 白名单 |
| `SetPaused` | 暂停/恢复程序 |
| `UpdateAdmin` | 更新管理员 |
| `SetFundProgram` | 设置 Fund Program |
| `AdminForceReleaseMargin` | 强制释放用户保证金 |
| `AdminPredictionMarketForceUnlock` | 强制释放预测市场锁定 |

---

## PDA 地址推导

### TypeScript 示例

```typescript
import { PublicKey } from '@solana/web3.js';

const VAULT_PROGRAM_ID = new PublicKey('vR3BifKCa2TGKP2uhToxZAMYAYydqpesvKGX54gzFny');

// VaultConfig PDA
const [vaultConfigPDA] = await PublicKey.findProgramAddress(
    [Buffer.from("vault_config")],
    VAULT_PROGRAM_ID
);

// UserAccount PDA
const [userAccountPDA] = await PublicKey.findProgramAddress(
    [Buffer.from("user"), userWallet.toBuffer()],
    VAULT_PROGRAM_ID
);

// PredictionMarketUserAccount PDA
const [pmUserAccountPDA] = await PublicKey.findProgramAddress(
    [Buffer.from("prediction_market_user"), userWallet.toBuffer()],
    VAULT_PROGRAM_ID
);
```

### Rust 示例

```rust
use solana_program::pubkey::Pubkey;

let (vault_config_pda, _bump) = Pubkey::find_program_address(
    &[b"vault_config"],
    &program_id,
);

let (user_account_pda, _bump) = Pubkey::find_program_address(
    &[b"user", user_wallet.as_ref()],
    &program_id,
);
```

---

## CPI 集成指南

### 从 Ledger Program 调用 LockMargin

```rust
use solana_program::program::invoke;

// 构建 CPI 调用
let lock_margin_ix = VaultInstruction::LockMargin { 
    amount: margin_amount 
};

invoke(
    &Instruction {
        program_id: vault_program_id,
        accounts: vec![
            AccountMeta::new_readonly(vault_config.key(), false),
            AccountMeta::new(user_account.key(), false),
            AccountMeta::new_readonly(*program_id, false), // 调用方 Program
        ],
        data: lock_margin_ix.try_to_vec()?,
    },
    &[vault_config, user_account],
)?;
```

### 验证调用方

Vault Program 内部验证:

```rust
fn verify_cpi_caller(
    config: &VaultConfig, 
    caller_program: &Pubkey
) -> Result<(), VaultError> {
    if config.is_authorized_caller(caller_program) {
        Ok(())
    } else {
        Err(VaultError::UnauthorizedCaller)
    }
}
```

---

## 安全机制

### 1. CPI 白名单验证

所有 CPI 指令都会验证调用方是否在白名单中：

```rust
if !vault_config.is_authorized_caller(caller_program.key) {
    return Err(VaultError::UnauthorizedCaller);
}
```

### 2. 余额安全

- 使用 `i64` 类型支持负数（未实现亏损）
- 所有运算使用 `checked_` 方法防止溢出
- 提款前验证可用余额充足

### 3. 暂停机制

- Admin 可随时暂停程序
- 暂停状态下禁止所有用户操作
- 紧急情况下的保护措施

### 4. Relayer 安全

- 仅 Admin 可执行 Relayer 操作
- 跨链消息由后端验证
- 出金需确保链下资金到位

---

## 构建与部署

### 构建

```bash
cd 1024-vault-program

# 编译检查
cargo check

# 运行测试
cargo test --lib

# 构建 BPF 程序
cargo build-sbf
```

### 部署

```bash
# 部署到 1024Chain Testnet
solana program deploy target/deploy/vault_program.so \
    --url https://testnet-rpc.1024chain.com/rpc/ \
    --program-id vR3BifKCa2TGKP2uhToxZAMYAYydqpesvKGX54gzFny \
    --use-rpc
```

---

## 测试

### 单元测试覆盖

| 测试项 | 文件 | 状态 |
|--------|------|------|
| UserAccount equity 计算 | `state.rs` | ✅ |
| VaultConfig authorized_caller | `state.rs` | ✅ |
| PredictionMarketUserAccount 锁定/释放 | `state.rs` | ✅ |
| 预测市场结算盈亏计算 | `state.rs` | ✅ |
| 安全数学运算 | `utils.rs` | ✅ |

### 运行测试

```bash
cargo test --lib

# 输出:
# running 6 tests
# test state::tests::test_user_account_equity ... ok
# test state::tests::test_vault_config_authorized_caller ... ok
# test state::tests::test_prediction_market_user_account_creation ... ok
# test state::tests::test_prediction_market_lock_unlock ... ok
# test state::tests::test_prediction_market_settle ... ok
# test state::tests::test_prediction_market_settle_with_profit ... ok
```

---

## 错误代码

| 错误 | Code | 说明 |
|------|------|------|
| `InsufficientBalance` | 0 | 余额不足 |
| `InvalidAmount` | 1 | 无效金额 |
| `UnauthorizedCaller` | 2 | 未授权的 CPI 调用方 |
| `VaultPaused` | 3 | Vault 已暂停 |
| `Overflow` | 4 | 数值溢出 |
| `Underflow` | 5 | 数值下溢 |
| `InvalidPDA` | 6 | 无效的 PDA 地址 |
| `InvalidAdmin` | 7 | 非管理员调用 |
| `AccountNotInitialized` | 8 | 账户未初始化 |
| `InvalidMint` | 9 | 无效的 Mint 地址 |

---

## 文件结构

```
1024-vault-program/
├── Cargo.toml
├── README.md
├── rust-toolchain.toml
└── src/
    ├── lib.rs          # 程序入口点
    ├── state.rs        # 账户结构定义
    ├── instruction.rs  # 指令枚举定义
    ├── processor.rs    # 指令处理逻辑
    ├── error.rs        # 错误类型
    ├── utils.rs        # 工具函数
    └── cpi.rs          # CPI Helper 函数
```

---

## License

MIT

---

*Last Updated: 2025-12-09*
