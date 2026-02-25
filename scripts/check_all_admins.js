/**
 * Check admin pubkey for ALL on-chain program config PDAs
 * 
 * Each config PDA stores the admin pubkey at a known offset.
 * For Borsh-serialized structs, the admin field is typically
 * the first Pubkey (32 bytes) after the account discriminator.
 */

const { Connection, PublicKey } = require('@solana/web3.js');

const RPC_URL = 'https://rpc.1024chain.com';
const EXPECTED_ADMIN = 'Ey5dD4Qzb5E8PM1aEh33CHgbVnvh8PuMVT9vaZwcCk5D'; // relayer1

const CONFIGS = [
  {
    name: 'Vault',
    program: 'C3pDwbciRtrxDr2Qfuqw67EUb9DHBJsAnmhty1jfk9fF',
    configPda: 'DMjPWD5GBzxfeSeqiscqN3hNiJqzsk5QBJqkXv9tQjeC',
  },
  {
    name: 'Exchange Ledger',
    program: 'HxRwFXgocnAcd2bFqmbPaahtWbpnVq7epjBH619GHmW9',
    configPda: '3suH3cRwMhoWTSfZ7N5XAL3qzHJubFtpxUV8DNZ4Gj8q',
  },
  {
    name: 'Prediction Market',
    program: 'ATxUQPdVbd8jDCN44qwitc8ojkF1cinhbCmcvmYWs9mq',
    configPda: '3SsW8RAxmCc1ZutpdzXG8fCBLX29WgMtwwBDrxWN51Bf',
  },
  {
    name: 'Fund',
    program: '35wKdvZ48HDu1rnpJEYkXufeGkNpsW1L25ZAmse7PUyM',
    configPda: '8jifjta1C9M9F5EjaPsY2ak7PaTo6yn36eqEjcsjGiiy',
  },
  {
    name: 'Listing',
    program: 'HqDhtezfvhMJyqTy2ZX4pf311kTBzbufjM2wqDmbtq2w',
    configPda: 'H8LE94mFYVpphjAyPXf6bc1RRZSBcVWfqjaDgVfwcQL4',
  },
  {
    name: 'Relayer Config',
    program: 'C3pDwbciRtrxDr2Qfuqw67EUb9DHBJsAnmhty1jfk9fF',
    configPda: 'FDsvVHYZ1HVyUHUfx2fCX6xAPi4yiHGk5zJCEBXWk9Aa',
  },
  {
    name: 'Insurance Fund Config',
    program: 'C3pDwbciRtrxDr2Qfuqw67EUb9DHBJsAnmhty1jfk9fF',
    configPda: 'AKorCbEwThniCPCxXLS9cnUfRw3P3Am5KsbGxYMAwmGd',
  },
  {
    name: 'PM Fee Config',
    program: 'ATxUQPdVbd8jDCN44qwitc8ojkF1cinhbCmcvmYWs9mq',
    configPda: 'D8rwT8xYqQN1z6XBtzdpAWKxyyigT4kHoH3iLFUeyQ4K',
  },
];

const FAUCET_PUBKEY = 'EdwhYb4DhUwymHbtJnVTwbnUCXejmUMrCXG7UnWsfXXE';

async function main() {
  const connection = new Connection(RPC_URL, 'confirmed');
  
  console.log('=== Checking ALL on-chain program admins ===');
  console.log(`Expected admin (relayer1): ${EXPECTED_ADMIN}`);
  console.log(`Old admin (faucet):        ${FAUCET_PUBKEY}`);
  console.log('');

  const results = [];

  for (const cfg of CONFIGS) {
    const pda = new PublicKey(cfg.configPda);
    try {
      const accountInfo = await connection.getAccountInfo(pda);
      if (!accountInfo) {
        results.push({ ...cfg, status: 'NOT_FOUND', admin: null });
        continue;
      }

      const data = accountInfo.data;
      
      // Scan the account data for known pubkeys (admin is typically in the first few fields)
      // Most Borsh-serialized config structs have admin as one of the first Pubkey fields
      const foundPubkeys = [];
      for (let offset = 0; offset <= data.length - 32; offset++) {
        const slice = data.slice(offset, offset + 32);
        const pubkey = new PublicKey(slice);
        const b58 = pubkey.toBase58();
        if (b58 === EXPECTED_ADMIN || b58 === FAUCET_PUBKEY) {
          foundPubkeys.push({ offset, pubkey: b58, isRelayer1: b58 === EXPECTED_ADMIN });
        }
      }

      if (foundPubkeys.length > 0) {
        const firstMatch = foundPubkeys[0];
        results.push({
          ...cfg,
          status: firstMatch.isRelayer1 ? 'OK' : 'NEEDS_UPDATE',
          admin: firstMatch.pubkey,
          offset: firstMatch.offset,
        });
      } else {
        // Try to read the first few pubkey-sized fields
        const firstPubkey = data.length >= 32 ? new PublicKey(data.slice(0, 32)).toBase58() : 'too_short';
        const secondPubkey = data.length >= 64 ? new PublicKey(data.slice(32, 64)).toBase58() : 'N/A';
        const thirdPubkey = data.length >= 96 ? new PublicKey(data.slice(64, 96)).toBase58() : 'N/A';
        results.push({
          ...cfg,
          status: 'UNKNOWN_ADMIN',
          admin: `field0=${firstPubkey}, field1=${secondPubkey}, field2=${thirdPubkey}`,
          offset: null,
        });
      }
    } catch (e) {
      results.push({ ...cfg, status: 'ERROR', admin: e.message });
    }
  }

  console.log('─'.repeat(90));
  console.log(
    'Program'.padEnd(25) +
    'Status'.padEnd(15) +
    'Admin'
  );
  console.log('─'.repeat(90));

  let needsUpdate = 0;
  for (const r of results) {
    const statusIcon = r.status === 'OK' ? '✅' : r.status === 'NEEDS_UPDATE' ? '❌' : '⚠️';
    console.log(
      `${statusIcon} ${r.name.padEnd(23)}${r.status.padEnd(15)}${r.admin || 'N/A'}`
    );
    if (r.status === 'NEEDS_UPDATE') needsUpdate++;
  }

  console.log('─'.repeat(90));
  if (needsUpdate === 0) {
    console.log('\n✅ All programs have admin set to relayer1!');
  } else {
    console.log(`\n❌ ${needsUpdate} program(s) still need admin update to relayer1`);
  }
}

main();
