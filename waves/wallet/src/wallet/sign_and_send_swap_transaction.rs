use crate::{
    esplora::broadcast,
    wallet::{current, get_txouts, Wallet},
};
use anyhow::Context;
use elements_fun::{secp256k1::SECP256K1, sighash::SigHashCache, Transaction, Txid};
use futures::lock::Mutex;
use swap::{alice_finalize_transaction, sign_with_key};
use wasm_bindgen::JsValue;

pub async fn sign_and_send_swap_transaction(
    name: String,
    current_wallet: &Mutex<Option<Wallet>>,
    transaction: Transaction,
) -> Result<Txid, JsValue> {
    let wallet = current(&name, current_wallet).await?;

    let txouts = get_txouts(&wallet, |utxo, txout| Ok(Some((utxo, txout)))).await?;

    let result = alice_finalize_transaction(transaction, |transaction| async {
        let mut cache = SigHashCache::new(&transaction);

        for (index, input, output) in
            transaction
                .clone()
                .input
                .iter_mut()
                .enumerate()
                .filter_map(|(index, input)| {
                    txouts
                        .iter()
                        .find(|(utxo, _)| {
                            utxo.txid == input.previous_output.txid
                                && utxo.vout == input.previous_output.vout
                        })
                        .map(|(_, txout)| (index, input, txout))
                })
        {
            input.witness.script_witness = sign_with_key(
                SECP256K1,
                &mut cache,
                index,
                &wallet.secret_key,
                output.as_confidential().unwrap().value,
            );
        }

        Ok(transaction)
    })
    .await
    .context("failed to finalize transaction as alice");

    let transaction = map_err_from_anyhow!(result)?;

    let txid = map_err_from_anyhow!(broadcast(transaction)
        .await
        .context("failed to broadcast transaction"))?;

    Ok(txid)
}
