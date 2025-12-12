# 1024 Vault Program

> 完全去中心化的资金托管程序 - 用户资金安全的核心保障

---

## 📋 目录

- [概述](#概述)
- [部署信息](#部署信息)
- [架构设计](#架构设计)
- [账户结构](#账户结构)
- [指令详解](#指令详解)
- [跨链桥集成](#跨链桥集成)
- [PDA 地址推导](#pda-地址推导)
- [CPI 集成指南](#cpi-集成指南)
- [安全机制](#安全机制)
- [构建与部署](#构建与部署)
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
| **跨链入金** | 支持 Arbitrum ↔ 1024Chain 跨链桥 |

---

## 部署信息

### 当前版本 (V3)

| 网络 | Program ID | 部署日期 | 状态 |
|------|-----------|---------|------|
| **1024Chain Testnet** | `4HfWrrWGsEkZs7yNLX1AHdvtrRGh7VX9S2e92rGkVpyU` | 2025-12-12 | ✅ 生产使用 |
| Solana Devnet | `5RTrqdYjWeZ8G4yi7P8jBmH8h7rCBkbnNML2dw3jopYM` | 2025-12-12 | 🧪 测试用 |

### V3 新增功能

- ✅ **TransferFromRelay** (#23): 支持中转账户到用户的账本转账
- ✅ **RelayerWithdrawAndTransfer** (#22): 支持出金并转给Relayer
- ✅ **Arbitrum ↔ 1024Chain 跨链桥集成**
- ✅ **真金托管模式**（Token真实存储在Vault）

### 历史版本

| 版本 | Program ID | 状态 |
|------|-----------|------|
| V2 | `vR3BifKCa2TGKP2uhToxZAMYAYydqpesvKGX54gzFny` | ❌ 已弃用 |

### 网络配置

```bash
# 1024Chain Testnet
RPC: https://testnet-rpc.1024chain.com/rpc/
WebSocket: wss://testnet-rpc.1024chain.com/ws/
浏览器: https://testnet-scan.1024chain.com/
```

### 依赖关系

```
                    ┌─────────────────────┐
                    │   Vault Program V3  │
                    │   (资金托管 + 跨链)   │
                    └─────────┬───────────┘
                              │
        ┌─────────────────────┼─────────────────────┐
        │                     │                     │
        ▼                     ▼                     ▼
┌───────────────┐   ┌─────────────────┐   ┌─────────────────┐
│ Ledger Program│   │  Fund Program   │   │ Bridge Program  │
│ (CPI 调用)     │   │ (保险基金)       │   │ (跨链桥)         │
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
│       │  Deposit / RelayerDeposit / TransferFromRelay           │
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

### CPI 操作指令

> ⚠️ 这些指令只能由白名单中的 Program 通过 CPI 调用

#### 4. LockMargin

锁定用户保证金（开仓时）。

```rust
LockMargin { amount: u64 }
```

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

## 跨链桥集成

### V3 新增跨链桥指令

#### 22. RelayerWithdrawAndTransfer

Relayer 执行出金操作，从用户账户扣款并转Token给Relayer。

```rust
RelayerWithdrawAndTransfer {
    user_wallet: Pubkey,  // 用户钱包地址
    amount: u64,          // 出金金额
}
```

**账户:**

| 账户 | 类型 | 说明 |
|------|------|------|
| 0 | `[signer]` | Relayer/Admin |
| 1 | `[writable]` | UserAccount PDA |
| 2 | `[writable]` | Vault Token Account |
| 3 | `[writable]` | Relayer Token Account |
| 4 | `[]` | VaultConfig |
| 5 | `[]` | Token Program |

**功能:**
- 从用户 UserAccount 扣除余额（账本）
- 执行 Token 转账：Vault TA → Relayer TA
- Relayer 收到真实 USDC 后，通过 Bridge 跨链到 Arbitrum

#### 23. TransferFromRelay

从中转账户转账到目标用户（账本内转账）。

```rust
TransferFromRelay {
    target_user: Pubkey,  // 目标用户钱包
    amount: u64,          // 转账金额
}
```

**账户:**

| 账户 | 类型 | 说明 |
|------|------|------|
| 0 | `[signer]` | 中转账户（已授权） |
| 1 | `[writable]` | 中转账户的 UserAccount |
| 2 | `[writable]` | 目标用户的 UserAccount |
| 3 | `[]` | VaultConfig |
| 4 | `[]` | System Program |

**功能:**
- 从中转账户余额扣除
- 增加目标用户余额
- 纯账本操作，无Token转账
- 用于跨链入金的最后一步

### 跨链桥入金流程

```
用户 (Arbitrum)
  ↓ 转 USDC
Bridge 合约 (Arbitrum)
  ↓ emit StakeEvent
e2s-submitter
  ┌─ Step 1: Bridge.submit_signature
  │   ↓ Bridge Vault → 中转TA (真金)
  ┌─ Step 2: Vault.Deposit
  │   ↓ 中转TA → Vault TA (真金)
  └─ Step 3: Vault.TransferFromRelay (#23)
      ↓ 中转余额 → 目标用户余额 (账本)
目标用户 Vault 账本 ✅
```

### 跨链桥出金流程

```
用户 (MetaMask)
  ↓ EIP-191 签名
POST /api/v1/vault-bridge/withdraw
  ↓
Vault.RelayerWithdrawAndTransfer (#22)
  ↓ 扣用户余额（账本）
  ↓ token::transfer(Vault TA → Relayer TA)
Relayer 收到 USDC
  ↓
Bridge.execute_stake()
  ↓ emit StakeEvent
s2e Relayer
  ↓
Arbitrum 用户收款 ✅
```

---

## PDA 地址推导

### TypeScript 示例

```typescript
import { PublicKey } from '@solana/web3.js';

const VAULT_PROGRAM_ID = new PublicKey('4HfWrrWGsEkZs7yNLX1AHdvtrRGh7VX9S2e92rGkVpyU');

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

### 4. 跨链桥安全

- 中转账户必须在 authorized_callers 白名单中
- TransferFromRelay 只能由授权账户调用
- 真金托管：Token 真实存储在 Vault Token Account

---

## 构建与部署

### 构建

```bash
cd 1024-vault-program

# 编译检查
cargo check

# 运行测试
cargo test --lib

# 构建 SBF 程序
cargo build-sbf
```

### 部署到 1024Chain

```bash
# 配置网络
solana config set --url https://testnet-rpc.1024chain.com/rpc/ \
    --ws wss://testnet-rpc.1024chain.com/ws/

# 部署（使用已有的 Program Keypair）
solana program deploy target/deploy/vault_program.so \
    --keypair faucet.json \
    --program-id vault-program-keypair.json

# 验证部署
solana program show 4HfWrrWGsEkZs7yNLX1AHdvtrRGh7VX9S2e92rGkVpyU
```

### 升级程序

```bash
# 升级使用相同的 Program Keypair
solana program deploy target/deploy/vault_program.so \
    --keypair faucet.json \
    --program-id vault-program-keypair.json \
    --upgrade-authority 267TEwwHkJUHz42TLNggDCecNhYHFxcRALmR17bPkvU8
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
├── vault-program-keypair.json    # Program Keypair (V3)
└── src/
    ├── lib.rs                     # 程序入口点
    ├── state.rs                   # 账户结构定义
    ├── instruction.rs             # 指令枚举定义 (24个指令)
    ├── processor.rs               # 指令处理逻辑
    ├── error.rs                   # 错误类型
    ├── utils.rs                   # 工具函数
    └── cpi.rs                     # CPI Helper 函数
```

---

## 相关文档

- **跨链桥开发文档**: `/docs/evm-adapter.md`
- **1024Chain配置**: `/当前配置信息.md`
- **部署总结**: `/vault-v3-deployment-summary.md`

---

## License

MIT

---

**当前版本**: V3  
**Program ID**: `4HfWrrWGsEkZs7yNLX1AHdvtrRGh7VX9S2e92rGkVpyU`  
**最后更新**: 2025-12-12
