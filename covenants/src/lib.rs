use std::future::Future;

use anyhow::{bail, Context, Result};
use elements::{
    bitcoin::{util::psbt::serialize::Serialize, Amount, Network, PrivateKey, PublicKey},
    confidential::{Asset, Nonce, Value},
    encode::Encodable,
    hashes::{sha256d, Hash},
    opcodes::all::*,
    script::Builder,
    secp256k1::{
        rand::{thread_rng, CryptoRng, RngCore},
        Secp256k1, SecretKey, Signature, Signing, Verification, SECP256K1,
    },
    sighash::SigHashCache,
    Address, AddressParams, AssetId, ConfidentialTxOut, OutPoint, Script, SigHashType, Transaction,
    TxIn, TxInWitness, TxOut, TxOutWitness, UnblindedTxOut,
};

/// These constants have been reverse engineered through the following transactions:
///
/// https://blockstream.info/liquid/tx/a17f4063b3a5fdf46a7012c82390a337e9a0f921933dccfb8a40241b828702f2
/// https://blockstream.info/liquid/tx/d12ff4e851816908810c7abc839dd5da2c54ad24b4b52800187bee47df96dd5c
/// https://blockstream.info/liquid/tx/47e60a3bc5beed45a2cf9fb7a8d8969bab4121df98b0034fb0d44f6ed2d60c7d
///
/// This gives us the following set of linear equations:
///
/// - 1 in, 1 out, 1 fee = 1332
/// - 1 in, 2 out, 1 fee = 2516
/// - 2 in, 2 out, 1 fee = 2623
///
/// Which we can solve using wolfram alpha: https://www.wolframalpha.com/input/?i=1x+%2B+1y+%2B+1z+%3D+1332%2C+1x+%2B+2y+%2B+1z+%3D+2516%2C+2x+%2B+2y+%2B+1z+%3D+2623
pub mod avg_vbytes {
    pub const INPUT: u64 = 107;
    pub const OUTPUT: u64 = 1184;
    pub const FEE: u64 = 41;
}

/// Estimate the virtual size of a transaction based on the number of inputs and outputs.
pub fn estimate_virtual_size(number_of_inputs: u64, number_of_outputs: u64) -> u64 {
    number_of_inputs * avg_vbytes::INPUT + number_of_outputs * avg_vbytes::OUTPUT + avg_vbytes::FEE
}

#[cfg(test)]
mod protocol_tests;

pub struct LoanRequest {
    collateral_amount: Amount,
    collateral_inputs: Vec<Input>,
    fee_sats_per_vbyte: Amount,
    borrower_pk: PublicKey,
    timelock: u64,
    borrower_address: Address,
}

pub struct LoanResponse {
    transaction: Transaction,
    collateral_blinding_sk: SecretKey,
    lender_pk: PublicKey,
    lender_address: Address,
    timelock: u64,
}

pub struct Borrower0 {
    keypair: (SecretKey, PublicKey),
    address: Address,
    address_blinding_sk: SecretKey,
    collateral_amount: Amount,
    collateral_inputs: Vec<Input>,
    fee_sats_per_vbyte: Amount,
    timelock: u64,
    bitcoin_asset_id: AssetId,
    usdt_asset_id: AssetId,
}

impl Borrower0 {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        address: Address,
        address_blinding_sk: SecretKey,
        collateral_amount: Amount,
        collateral_inputs: Vec<Input>,
        fee_sats_per_vbyte: Amount,
        timelock: u64,
        bitcoin_asset_id: AssetId,
        usdt_asset_id: AssetId,
    ) -> Result<Self> {
        let keypair = make_keypair();

        Ok(Self {
            keypair,
            address,
            address_blinding_sk,
            collateral_amount,
            collateral_inputs,
            fee_sats_per_vbyte,
            timelock,
            bitcoin_asset_id,
            usdt_asset_id,
        })
    }

    pub fn loan_request(&self) -> LoanRequest {
        LoanRequest {
            collateral_amount: self.collateral_amount,
            collateral_inputs: self.collateral_inputs.clone(),
            fee_sats_per_vbyte: self.fee_sats_per_vbyte,
            borrower_pk: self.keypair.1,
            timelock: self.timelock,
            borrower_address: self.address.clone(),
        }
    }

    pub fn interpret<C>(self, secp: &Secp256k1<C>, loan_response: LoanResponse) -> Result<Borrower1>
    where
        C: Verification,
    {
        let transaction = loan_response.transaction;

        let principal_tx_out_amount = transaction
            .output
            .iter()
            .find_map(|out| match out.to_confidential() {
                Some(conf) => {
                    let unblinded_out = conf.unblind(secp, self.address_blinding_sk).ok()?;
                    let predicate = unblinded_out.asset == self.usdt_asset_id
                        && conf.script_pubkey == self.address.script_pubkey();

                    predicate.then(|| Amount::from_sat(unblinded_out.value))
                }
                None => None,
            })
            .context("no principal txout")?;

        let (collateral_script, repayment_tx_out) = loan_contract(
            self.keypair.1,
            loan_response.lender_pk,
            principal_tx_out_amount,
            &loan_response.lender_address,
            loan_response.timelock,
            self.usdt_asset_id,
        );
        let collateral_address = Address::p2wsh(&collateral_script, None, &AddressParams::ELEMENTS);
        let collateral_script_pubkey = collateral_address.script_pubkey();
        let collateral_blinding_sk = loan_response.collateral_blinding_sk;
        transaction
            .output
            .iter()
            .find_map(|out| match out.to_confidential() {
                Some(conf) => {
                    let unblinded_out = conf.unblind(secp, collateral_blinding_sk).ok()?;
                    let predicate = unblinded_out.asset == self.bitcoin_asset_id
                        && unblinded_out.value == self.collateral_amount.as_sat()
                        && out.script_pubkey == collateral_script_pubkey;

                    predicate.then(|| out)
                }
                None => None,
            })
            .context("no collateral txout")?;

        let collateral_input_amount = self
            .collateral_inputs
            .iter()
            .map(|input| input.clone().into_unblinded_input(secp))
            .try_fold(0, |sum, input| {
                input.map(|input| sum + input.unblinded.value).ok()
            })
            .context("could not sum collateral inputs")?;
        let tx_fee = Amount::from_sat(
            estimate_virtual_size(transaction.input.len() as u64, 4)
                * self.fee_sats_per_vbyte.as_sat(),
        );
        let collateral_change_amount = Amount::from_sat(collateral_input_amount)
            .checked_sub(self.collateral_amount)
            .map(|a| a.checked_sub(tx_fee))
            .flatten()
            .with_context(|| {
                format!(
                    "cannot pay for output {} and fee {} with input {}",
                    self.collateral_amount, tx_fee, collateral_input_amount,
                )
            })?;

        transaction
            .output
            .iter()
            .find_map(|out| match out.to_confidential() {
                Some(conf) => {
                    let unblinded_out = conf.unblind(secp, self.address_blinding_sk).ok()?;
                    let predicate = unblinded_out.asset == self.bitcoin_asset_id
                        && unblinded_out.value == collateral_change_amount.as_sat()
                        && out.script_pubkey == self.address.script_pubkey();

                    predicate.then(|| out)
                }
                None => None,
            })
            .context("no collateral change txout")?;

        Ok(Borrower1 {
            keypair: self.keypair,
            loan_transaction: transaction,
            collateral_amount: self.collateral_amount,
            collateral_script,
            principal_tx_out_amount,
            address: self.address,
            repayment_tx_out,
            bitcoin_asset_id: self.bitcoin_asset_id,
            usdt_asset_id: self.usdt_asset_id,
        })
    }
}

pub struct Borrower1 {
    keypair: (SecretKey, PublicKey),
    loan_transaction: Transaction,
    collateral_amount: Amount,
    collateral_script: Script,
    principal_tx_out_amount: Amount,
    address: Address,
    repayment_tx_out: TxOut,
    bitcoin_asset_id: AssetId,
    usdt_asset_id: AssetId,
}

impl Borrower1 {
    pub async fn sign<S, F>(&self, signer: S) -> Result<Transaction>
    where
        S: FnOnce(Transaction) -> F,
        F: Future<Output = Result<Transaction>>,
    {
        signer(self.loan_transaction.clone()).await
    }

    pub async fn loan_repayment_transaction<C, CF, S, SF>(
        &self,
        coin_selector: C,
        signer: S,
        tx_fee: Amount,
    ) -> Result<Transaction>
    where
        C: FnOnce(Amount, AssetId) -> CF,
        CF: Future<Output = Result<Vec<UnblindedInput>>>,
        S: FnOnce(Transaction) -> SF,
        SF: Future<Output = Result<Transaction>>,
    {
        let loan_transaction = self.loan_transaction.clone();
        let loan_txid = loan_transaction.txid();

        // construct collateral input
        let collateral_address =
            Address::p2wsh(&self.collateral_script, None, &AddressParams::ELEMENTS);
        let collateral_script_pubkey = collateral_address.script_pubkey();
        let vout = self
            .loan_transaction
            .output
            .iter()
            .position(|out| out.script_pubkey == collateral_script_pubkey)
            .context("no collateral txout")?;

        let collateral_input = TxIn {
            previous_output: OutPoint {
                txid: loan_txid,
                vout: vout as u32,
            },
            is_pegin: false,
            has_issuance: false,
            script_sig: Default::default(),
            sequence: 0,
            asset_issuance: Default::default(),
            witness: Default::default(),
        };

        // construct repayment input and repayment change output
        let (mut repayment_inputs, repayment_change) = {
            let repayment_amount = self.principal_tx_out_amount;
            let inputs = coin_selector(repayment_amount, self.usdt_asset_id).await?;

            let input_amount = inputs
                .iter()
                .fold(0, |acc, input| acc + input.unblinded.value);
            let inputs = inputs.into_iter().map(|input| input.tx_in).collect();

            let change_amount = Amount::from_sat(input_amount)
                .checked_sub(repayment_amount)
                .with_context(|| {
                    format!(
                        "cannot pay for output {} with input {}",
                        repayment_amount, input_amount,
                    )
                })?;

            let change_output = match change_amount {
                Amount::ZERO => None,
                _ => Some(TxOut {
                    asset: Asset::Explicit(self.usdt_asset_id),
                    value: Value::Explicit(change_amount.as_sat()),
                    nonce: Nonce::Null,
                    script_pubkey: self.address.script_pubkey(),
                    witness: TxOutWitness::default(),
                }),
            };

            (inputs, change_output)
        };

        let collateral_output = TxOut {
            asset: Asset::Explicit(self.bitcoin_asset_id),
            value: Value::Explicit((self.collateral_amount - tx_fee).as_sat()),
            nonce: Default::default(),
            script_pubkey: self.address.script_pubkey(),
            witness: Default::default(),
        };

        let tx_fee_output = TxOut {
            asset: Asset::Explicit(self.bitcoin_asset_id),
            value: Value::Explicit(tx_fee.as_sat()),
            nonce: Default::default(),
            script_pubkey: Default::default(),
            witness: Default::default(),
        };

        let mut tx_ins = vec![collateral_input];
        tx_ins.append(&mut repayment_inputs);

        let mut tx_outs = vec![
            self.repayment_tx_out.clone(),
            collateral_output,
            tx_fee_output,
        ];
        if let Some(repayment_change) = repayment_change {
            tx_outs.push(repayment_change)
        }

        let mut tx = Transaction {
            version: 2,
            lock_time: 0,
            input: tx_ins,
            output: tx_outs,
        };

        // fulfill collateral input covenant script
        {
            let sighash = SigHashCache::new(&tx).segwitv0_sighash(
                0,
                &self.collateral_script.clone(),
                Value::Explicit(self.collateral_amount.as_sat()),
                SigHashType::All,
            );

            let sig = SECP256K1.sign(
                &elements::secp256k1::Message::from(sighash),
                &self.keypair.0,
            );

            tx.input[0].witness = TxInWitness {
                amount_rangeproof: vec![],
                inflation_keys_rangeproof: vec![],
                script_witness: RepaymentWitnessStack::new(
                    sig,
                    self.keypair.1,
                    self.collateral_amount.as_sat(),
                    &tx,
                    self.collateral_script.clone(),
                )
                .unwrap()
                .serialise()
                .unwrap(),
                pegin_witness: vec![],
            };
        };

        let tx = signer(tx).await?;

        Ok(tx)
    }
}

pub struct Lender0 {
    keypair: (SecretKey, PublicKey),
    principal_inputs: Vec<UnblindedInput>,
    address: Address,
    bitcoin_asset_id: AssetId,
    usdt_asset_id: AssetId,
}

impl Lender0 {
    pub fn new<C>(
        secp: &Secp256k1<C>,
        bitcoin_asset_id: AssetId,
        usdt_asset_id: AssetId,
        // TODO: Here we assume that the wallet is giving us _all_ the
        // inputs available. It would be better to coin-select these
        // as soon as we know the principal amount after receiving the
        // loan request
        principal_inputs: Vec<Input>,
        address: Address,
    ) -> Result<Self>
    where
        C: Verification,
    {
        let keypair = make_keypair();

        let principal_inputs = principal_inputs
            .into_iter()
            .map(|input| input.into_unblinded_input(secp))
            .collect::<Result<Vec<_>>>()?;

        Ok(Self {
            bitcoin_asset_id,
            keypair,
            address,
            usdt_asset_id,
            principal_inputs,
        })
    }

    pub fn interpret<R, C>(
        self,
        rng: &mut R,
        secp: &Secp256k1<C>,
        loan_request: LoanRequest,
    ) -> Result<Lender1>
    where
        R: RngCore + CryptoRng,
        C: Verification + Signing,
    {
        let principal_amount = Lender0::calc_principal_amount(&loan_request);
        let collateral_inputs = loan_request
            .collateral_inputs
            .into_iter()
            .map(|input| input.into_unblinded_input(secp))
            .collect::<Result<Vec<_>>>()?;

        let borrower_inputs = collateral_inputs.iter().map(|input| {
            (
                input.unblinded.asset,
                input.unblinded.value,
                input.confidential.asset,
                input.unblinded.asset_blinding_factor,
                input.unblinded.value_blinding_factor,
            )
        });
        let lender_inputs = self.principal_inputs.iter().map(|input| {
            (
                input.unblinded.asset,
                input.unblinded.value,
                input.confidential.asset,
                input.unblinded.asset_blinding_factor,
                input.unblinded.value_blinding_factor,
            )
        });

        let inputs = borrower_inputs.chain(lender_inputs).collect::<Vec<_>>();

        let collateral_input_amount = collateral_inputs
            .iter()
            .fold(0, |sum, input| sum + input.unblinded.value);

        let collateral_amount = loan_request.collateral_amount;

        let (_, lender_pk) = self.keypair;
        let (collateral_script, _) = loan_contract(
            loan_request.borrower_pk,
            lender_pk,
            principal_amount,
            &self.address,
            loan_request.timelock,
            self.usdt_asset_id,
        );

        let (collateral_blinding_sk, collateral_blinding_pk) = make_keypair();
        let collateral_address = Address::p2wsh(
            &collateral_script,
            Some(collateral_blinding_pk.key),
            &AddressParams::ELEMENTS,
        );
        let (collateral_tx_out, abf_collateral, vbf_collateral) = TxOut::new_not_last_confidential(
            rng,
            secp,
            dbg!(collateral_amount.as_sat()),
            collateral_address,
            dbg!(self.bitcoin_asset_id),
            &inputs,
        )
        .context("could not construct collateral txout")?;

        let (principal_tx_out, abf_principal, vbf_principal) = TxOut::new_not_last_confidential(
            rng,
            secp,
            dbg!(principal_amount.as_sat()),
            loan_request.borrower_address.clone(),
            dbg!(self.usdt_asset_id),
            &inputs,
        )
        .context("could not construct principal txout")?;

        let principal_input_amount = self
            .principal_inputs
            .iter()
            .fold(0, |sum, input| sum + input.unblinded.value);
        let principal_change_amount =
            Amount::from_sat(dbg!(principal_input_amount)) - principal_amount;
        let (principal_change_tx_out, abf_principal_change, vbf_principal_change) =
            TxOut::new_not_last_confidential(
                rng,
                secp,
                dbg!(principal_change_amount).as_sat(),
                self.address.clone(),
                dbg!(self.usdt_asset_id),
                &inputs,
            )
            .context("could not construct principal change txout")?;

        let not_last_confidential_outputs = [
            (collateral_amount.as_sat(), abf_collateral, vbf_collateral),
            (principal_amount.as_sat(), abf_principal, vbf_principal),
            (
                principal_change_amount.as_sat(),
                abf_principal_change,
                vbf_principal_change,
            ),
        ];

        let tx_fee = Amount::from_sat(
            estimate_virtual_size(inputs.len() as u64, 4)
                * loan_request.fee_sats_per_vbyte.as_sat(),
        );
        let collateral_change_amount = Amount::from_sat(dbg!(collateral_input_amount))
            .checked_sub(collateral_amount)
            .map(|a| a.checked_sub(dbg!(tx_fee)))
            .flatten()
            .with_context(|| {
                format!(
                    "cannot pay for output {} and fee {} with input {}",
                    collateral_amount, tx_fee, collateral_input_amount,
                )
            })?;
        let collateral_change_tx_out = TxOut::new_last_confidential(
            rng,
            secp,
            dbg!(collateral_change_amount.as_sat()),
            loan_request.borrower_address,
            dbg!(self.bitcoin_asset_id),
            &inputs,
            &not_last_confidential_outputs,
        )?;

        let tx_ins = {
            let borrower_inputs = collateral_inputs.iter().map(|input| input.tx_in.clone());
            let lender_inputs = self
                .principal_inputs
                .iter()
                .map(|input| input.tx_in.clone());
            borrower_inputs.chain(lender_inputs).collect::<Vec<_>>()
        };

        let tx_fee_tx_out = TxOut::new_fee(tx_fee.as_sat(), self.bitcoin_asset_id);

        let loan_transaction = Transaction {
            version: 2,
            lock_time: 0,
            input: tx_ins,
            output: vec![
                collateral_tx_out,
                principal_tx_out,
                principal_change_tx_out,
                collateral_change_tx_out,
                tx_fee_tx_out,
            ],
        };

        Ok(Lender1 {
            keypair: self.keypair,
            address: self.address,
            timelock: loan_request.timelock,
            loan_transaction,
            collateral_script,
            collateral_amount: loan_request.collateral_amount,
            collateral_blinding_sk,
            bitcoin_asset_id: self.bitcoin_asset_id,
        })
    }

    fn calc_principal_amount(loan_request: &LoanRequest) -> Amount {
        Amount::from_sat(loan_request.collateral_amount.as_sat() / 2)
    }
}

pub struct Lender1 {
    keypair: (SecretKey, PublicKey),
    address: Address,
    timelock: u64,
    loan_transaction: Transaction,
    collateral_script: Script,
    collateral_amount: Amount,
    collateral_blinding_sk: SecretKey,
    bitcoin_asset_id: AssetId,
}

impl Lender1 {
    pub fn loan_response(&self) -> LoanResponse {
        LoanResponse {
            transaction: self.loan_transaction.clone(),
            collateral_blinding_sk: self.collateral_blinding_sk,
            lender_pk: self.keypair.1,
            lender_address: self.address.clone(),
            timelock: self.timelock,
        }
    }

    pub async fn finalise_loan<S, F>(
        &self,
        loan_transaction: Transaction,
        signer: S,
    ) -> Result<Transaction>
    where
        S: FnOnce(Transaction) -> F,
        F: Future<Output = Result<Transaction>>,
    {
        if self.loan_transaction.txid() != loan_transaction.txid() {
            bail!("wrong loan transaction")
        }

        signer(loan_transaction).await
    }

    pub fn liquidation_transaction(&self, tx_fee: Amount) -> Result<Transaction> {
        let loan_transaction = self.loan_transaction.clone();
        let loan_txid = loan_transaction.txid();

        // construct collateral input
        let collateral_address =
            Address::p2wsh(&self.collateral_script, None, &AddressParams::ELEMENTS);
        let collateral_script_pubkey = collateral_address.script_pubkey();
        let vout = self
            .loan_transaction
            .output
            .iter()
            .position(|out| out.script_pubkey == collateral_script_pubkey)
            .context("no collateral txout")?;

        let collateral_input = TxIn {
            previous_output: OutPoint {
                txid: loan_txid,
                vout: vout as u32,
            },
            is_pegin: false,
            has_issuance: false,
            script_sig: Default::default(),
            sequence: 0,
            asset_issuance: Default::default(),
            witness: Default::default(),
        };

        let collateral_tx_out = TxOut {
            asset: Asset::Explicit(self.bitcoin_asset_id),
            value: Value::Explicit((self.collateral_amount - tx_fee).as_sat()),
            nonce: Nonce::Null,
            script_pubkey: self.address.script_pubkey(),
            witness: TxOutWitness::default(),
        };

        let tx_fee_tx_out = TxOut::new_fee(tx_fee.as_sat(), self.bitcoin_asset_id);

        let mut liquidation_transaction = Transaction {
            version: 2,
            lock_time: 0,
            input: vec![collateral_input],
            output: vec![collateral_tx_out, tx_fee_tx_out],
        };

        {
            let sighash = SigHashCache::new(&liquidation_transaction).segwitv0_sighash(
                0,
                &self.collateral_script.clone(),
                Value::Explicit(self.collateral_amount.as_sat()),
                SigHashType::All,
            );

            let sig = SECP256K1.sign(
                &elements::secp256k1::Message::from(sighash),
                &self.keypair.0,
            );
            let mut sig = sig.serialize_der().to_vec();
            sig.push(SigHashType::All as u8);

            let if_flag = vec![];

            liquidation_transaction.input[0].witness = TxInWitness {
                amount_rangeproof: vec![],
                inflation_keys_rangeproof: vec![],
                script_witness: vec![sig, if_flag, self.collateral_script.to_bytes()],
                pegin_witness: vec![],
            };
        }

        Ok(liquidation_transaction)
    }
}

fn loan_contract(
    borrower_pk: PublicKey,
    lender_pk: PublicKey,
    principal_amount: Amount,
    lender_address: &Address,
    timelock: u64,
    usdt_asset_id: AssetId,
) -> (Script, TxOut) {
    let repayment_output = TxOut {
        asset: Asset::Explicit(usdt_asset_id),
        value: Value::Explicit(principal_amount.as_sat()),
        nonce: Default::default(),
        script_pubkey: lender_address.script_pubkey(),
        witness: Default::default(),
    };

    let mut repayment_output_bytes = Vec::new();
    repayment_output
        .consensus_encode(&mut repayment_output_bytes)
        .unwrap();

    let script = Builder::new()
        .push_opcode(OP_IF)
        .push_opcode(OP_DEPTH)
        .push_opcode(OP_1SUB)
        .push_opcode(OP_PICK)
        .push_opcode(OP_PUSHNUM_1)
        .push_opcode(OP_CAT)
        .push_slice(&borrower_pk.serialize())
        .push_opcode(OP_CHECKSIGVERIFY)
        .push_slice(repayment_output_bytes.as_slice())
        .push_opcode(OP_2ROT)
        .push_int(5)
        .push_opcode(OP_ROLL)
        .push_opcode(OP_CAT)
        .push_opcode(OP_CAT)
        .push_opcode(OP_CAT)
        .push_opcode(OP_HASH256)
        .push_opcode(OP_ROT)
        .push_opcode(OP_ROT)
        .push_opcode(OP_CAT)
        .push_opcode(OP_CAT)
        .push_opcode(OP_CAT)
        .push_opcode(OP_CAT)
        .push_opcode(OP_CAT)
        .push_opcode(OP_CAT)
        .push_opcode(OP_CAT)
        .push_opcode(OP_CAT)
        .push_opcode(OP_CAT)
        .push_opcode(OP_CAT)
        .push_opcode(OP_CAT)
        .push_opcode(OP_CAT)
        .push_opcode(OP_SHA256)
        .push_opcode(OP_SWAP)
        .push_opcode(OP_CHECKSIGFROMSTACK)
        .push_opcode(OP_ELSE)
        .push_int(timelock as i64)
        .push_opcode(OP_CLTV)
        .push_opcode(OP_DROP)
        .push_slice(&lender_pk.serialize())
        .push_opcode(OP_CHECKSIG)
        .push_opcode(OP_ENDIF)
        .into_script();

    (script, repayment_output)
}

struct RepaymentWitnessStack {
    sig: Signature,
    pk: PublicKey,
    tx_version: u32,
    hash_prev_out: sha256d::Hash,
    hash_sequence: sha256d::Hash,
    hash_issuances: sha256d::Hash,
    input: InputData,
    other_outputs: Vec<TxOut>,
    lock_time: u32,
    sighash_type: SigHashType,
}

struct InputData {
    previous_output: OutPoint,
    script: Script,
    value: Value,
    sequence: u32,
}

impl RepaymentWitnessStack {
    fn new(
        sig: Signature,
        pk: PublicKey,
        collateral_amount: u64,
        tx: &Transaction,
        script: Script,
    ) -> Result<Self> {
        let tx_version = tx.version;

        let hash_prev_out = {
            let mut enc = sha256d::Hash::engine();
            for txin in tx.input.iter() {
                txin.previous_output.consensus_encode(&mut enc)?;
            }

            sha256d::Hash::from_engine(enc)
        };

        let hash_sequence = {
            let mut enc = sha256d::Hash::engine();

            for txin in tx.input.iter() {
                txin.sequence.consensus_encode(&mut enc)?;
            }
            sha256d::Hash::from_engine(enc)
        };

        let hash_issuances = {
            let mut enc = sha256d::Hash::engine();
            for txin in tx.input.iter() {
                if txin.has_issuance() {
                    txin.asset_issuance.consensus_encode(&mut enc)?;
                } else {
                    0u8.consensus_encode(&mut enc)?;
                }
            }
            sha256d::Hash::from_engine(enc)
        };

        let input = {
            let input = &tx.input[0];
            let value = Value::Explicit(collateral_amount);
            InputData {
                previous_output: input.previous_output,
                script,
                value,
                sequence: input.sequence,
            }
        };

        let other_outputs = tx.output[1..].to_vec();

        let lock_time = tx.lock_time;

        let sighash_type = SigHashType::All;

        Ok(Self {
            sig,
            pk,
            tx_version,
            hash_prev_out,
            hash_sequence,
            hash_issuances,
            input,
            other_outputs,
            lock_time,
            sighash_type,
        })
    }

    fn serialise(&self) -> anyhow::Result<Vec<Vec<u8>>> {
        let if_flag = vec![0x01];

        let sig = self.sig.serialize_der().to_vec();

        let pk = self.pk.serialize().to_vec();

        let tx_version = {
            let mut writer = Vec::new();
            self.tx_version.consensus_encode(&mut writer)?;
            writer
        };

        // input specific values
        let (previous_out, script_0, script_1, script_2, value, sequence) = {
            let InputData {
                previous_output,
                script,
                value,
                sequence,
            } = &self.input;

            let third = script.len() / 3;

            (
                {
                    let mut writer = Vec::new();
                    previous_output.consensus_encode(&mut writer)?;
                    writer
                },
                {
                    let mut writer = Vec::new();
                    script.consensus_encode(&mut writer)?;
                    writer[..third].to_vec()
                },
                {
                    let mut writer = Vec::new();
                    script.consensus_encode(&mut writer)?;
                    writer[third..2 * third].to_vec()
                },
                {
                    let mut writer = Vec::new();
                    script.consensus_encode(&mut writer)?;
                    writer[2 * third..].to_vec()
                },
                {
                    let mut writer = Vec::new();
                    value.consensus_encode(&mut writer)?;
                    writer
                },
                {
                    let mut writer = Vec::new();
                    sequence.consensus_encode(&mut writer)?;
                    writer
                },
            )
        };

        // hashoutputs (only supporting SigHashType::All)
        let other_outputs = {
            let mut other_outputs = vec![];

            for txout in self.other_outputs.iter() {
                let mut output = Vec::new();
                txout.consensus_encode(&mut output)?;
                other_outputs.push(output)
            }

            if other_outputs.len() < 2 {
                bail!("insufficient outputs");
            }

            if other_outputs.len() == 2 {
                other_outputs.push(vec![])
            }

            other_outputs
        };

        let lock_time = {
            let mut writer = Vec::new();
            self.lock_time.consensus_encode(&mut writer)?;
            writer
        };

        let sighash_type = {
            let mut writer = Vec::new();
            self.sighash_type.as_u32().consensus_encode(&mut writer)?;
            writer
        };

        Ok(vec![
            sig,
            pk,
            tx_version,
            self.hash_prev_out.to_vec(),
            self.hash_sequence.to_vec(),
            self.hash_issuances.to_vec(),
            previous_out,
            script_0,
            script_1,
            script_2,
            value,
            sequence,
            other_outputs[0].clone(),
            other_outputs[1].clone(),
            other_outputs[2].clone(),
            lock_time,
            sighash_type,
            if_flag,
            self.input.script.clone().into_bytes(),
        ])
    }
}

#[derive(Debug, Clone)]
pub struct Input {
    pub tx_in: TxIn,
    pub tx_out: TxOut,
    pub blinding_key: SecretKey,
}

impl Input {
    fn into_unblinded_input<C>(self, secp: &Secp256k1<C>) -> Result<UnblindedInput>
    where
        C: Verification,
    {
        let tx_in = self.tx_in;
        let confidential = self
            .tx_out
            .into_confidential()
            .with_context(|| format!("input {} is not confidential", tx_in.previous_output))?;

        let unblinded = confidential.unblind(secp, self.blinding_key)?;

        Ok(UnblindedInput {
            tx_in,
            confidential,
            unblinded,
        })
    }
}

#[derive(Debug, Clone)]
pub struct UnblindedInput {
    pub tx_in: TxIn,
    pub confidential: ConfidentialTxOut,
    pub unblinded: UnblindedTxOut,
}

// TODO: Take rng param
fn make_keypair() -> (SecretKey, PublicKey) {
    let sk = SecretKey::new(&mut thread_rng());
    let pk = PublicKey::from_private_key(
        &SECP256K1,
        &PrivateKey {
            compressed: true,
            network: Network::Regtest,
            key: sk,
        },
    );

    (sk, pk)
}
