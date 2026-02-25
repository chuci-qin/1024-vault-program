/**
 * Update Prediction Market Program Admin
 * 
 * Changes the on-chain PMConfig admin from faucet.json (EdwhYb4D...)
 * to relayer1.json (Ey5dD4Qz...)
 */

const {
  Connection,
  Keypair,
  PublicKey,
  Transaction,
  TransactionInstruction,
  sendAndConfirmTransaction,
} = require('@solana/web3.js');
const fs = require('fs');

const RPC_URL = 'https://rpc.1024chain.com';
const PM_PROGRAM_ID = new PublicKey('ATxUQPdVbd8jDCN44qwitc8ojkF1cinhbCmcvmYWs9mq');
const PM_CONFIG_PDA = new PublicKey('3SsW8RAxmCc1ZutpdzXG8fCBLX29WgMtwwBDrxWN51Bf');

// Borsh enum index: count variants in PredictionMarketInstruction enum
// 0:Initialize 1:ReinitializeConfig 2:CreateMarket 3:ActivateMarket
// 4:PauseMarket 5:ResumeMarket 6:CancelMarket 7:FlagMarket
// 8:MintCompleteSet 9:RedeemCompleteSet 10:PlaceOrder 11:CancelOrder
// 12:MatchMint 13:MatchBurn 14:ExecuteTrade 15:ProposeResult
// 16:ChallengeResult 17:FinalizeResult 18:ResolveDispute
// 19:ClaimWinnings 20:RefundCancelledMarket 21:UpdateAdmin
const UPDATE_ADMIN_IX = 21;

const FAUCET_KEY_PATH = '/Users/chuciqin/Desktop/project1024/1024codebase/1024chain-config-production/keys/mainnet/node1/faucet.json';
const RELAYER1_KEY_PATH = '/Users/chuciqin/Desktop/project1024/1024codebase/1024chain-config-production/keys/mainnet/node1/relayers/relayer1.json';

async function main() {
  const connection = new Connection(RPC_URL, 'confirmed');

  const faucetSecret = JSON.parse(fs.readFileSync(FAUCET_KEY_PATH, 'utf8'));
  const currentAdmin = Keypair.fromSecretKey(Uint8Array.from(faucetSecret));

  const relayer1Secret = JSON.parse(fs.readFileSync(RELAYER1_KEY_PATH, 'utf8'));
  const newAdmin = Keypair.fromSecretKey(Uint8Array.from(relayer1Secret));

  console.log('=== Prediction Market UpdateAdmin ===');
  console.log(`PM Program:     ${PM_PROGRAM_ID.toBase58()}`);
  console.log(`PM Config:      ${PM_CONFIG_PDA.toBase58()}`);
  console.log(`Current Admin:  ${currentAdmin.publicKey.toBase58()}`);
  console.log(`New Admin:      ${newAdmin.publicKey.toBase58()}`);
  console.log('');

  // Borsh: [u8 variant_index] + [32 bytes new_admin pubkey]
  const data = Buffer.alloc(1 + 32);
  data.writeUInt8(UPDATE_ADMIN_IX, 0);
  newAdmin.publicKey.toBuffer().copy(data, 1);

  const ix = new TransactionInstruction({
    programId: PM_PROGRAM_ID,
    keys: [
      { pubkey: currentAdmin.publicKey, isSigner: true, isWritable: true },
      { pubkey: PM_CONFIG_PDA, isSigner: false, isWritable: true },
    ],
    data,
  });

  const tx = new Transaction().add(ix);

  try {
    const { blockhash } = await connection.getLatestBlockhash();
    tx.recentBlockhash = blockhash;
    tx.feePayer = currentAdmin.publicKey;

    const signature = await sendAndConfirmTransaction(connection, tx, [currentAdmin], {
      commitment: 'confirmed',
    });

    console.log('✅ PM UpdateAdmin successful!');
    console.log(`Signature: ${signature}`);
    console.log(`Admin changed: ${currentAdmin.publicKey.toBase58()} → ${newAdmin.publicKey.toBase58()}`);
  } catch (error) {
    console.error('❌ PM UpdateAdmin failed:');
    if (error.logs) {
      error.logs.forEach(log => console.error('  ', log));
    }
    console.error(error.message || error);
    process.exit(1);
  }
}

main();
