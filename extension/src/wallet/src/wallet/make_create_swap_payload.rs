use crate::{
    constants::{NATIVE_ASSET_ID, USDT_ASSET_ID},
    wallet::{
        coin_selection, coin_selection::coin_select, current, get_txouts, CreateSwapPayload,
        SwapUtxo, Wallet,
    },
};
use anyhow::{Context, Result};
use bdk::bitcoin::{Amount, Denomination};
use elements::{secp256k1::SECP256K1, AssetId, OutPoint};
use futures::lock::Mutex;
use swap::avg_vbytes;

pub async fn make_buy_create_swap_payload(
    name: String,
    current_wallet: &Mutex<Option<Wallet>>,
    sell_amount: String,
) -> Result<CreateSwapPayload> {
    make_create_swap_payload(
        name,
        current_wallet,
        sell_amount,
        *USDT_ASSET_ID,
        *NATIVE_ASSET_ID,
    )
    .await
}

pub async fn make_sell_create_swap_payload(
    name: String,
    current_wallet: &Mutex<Option<Wallet>>,
    sell_amount: String,
) -> Result<CreateSwapPayload> {
    make_create_swap_payload(
        name,
        current_wallet,
        sell_amount,
        *NATIVE_ASSET_ID,
        *NATIVE_ASSET_ID,
    )
    .await
}

async fn make_create_swap_payload(
    name: String,
    current_wallet: &Mutex<Option<Wallet>>,
    sell_amount: String,
    sell_asset: AssetId,
    fee_asset: AssetId,
) -> Result<CreateSwapPayload> {
    let sell_amount = Amount::from_str_in(&sell_amount, Denomination::Bitcoin)
        .context("failed to parse amount from string")?;

    let wallet = current(&name, current_wallet).await?;
    let blinding_key = wallet.blinding_key();

    let utxos = get_txouts(&wallet, |utxo, txout| {
        Ok(match txout.into_confidential() {
            Some(confidential) => {
                let unblinded_txout = confidential.unblind(SECP256K1, blinding_key)?;
                let outpoint = OutPoint {
                    txid: utxo.txid,
                    vout: utxo.vout,
                };
                let candidate_asset = unblinded_txout.asset;

                if candidate_asset == sell_asset {
                    Some(coin_selection::Utxo {
                        outpoint,
                        value: unblinded_txout.value,
                        script_pubkey: confidential.script_pubkey,
                        asset: candidate_asset,
                    })
                } else {
                    log::debug!(
                        "utxo {} with asset id {} is not the sell asset, ignoring",
                        outpoint,
                        candidate_asset
                    );
                    None
                }
            }
            None => {
                log::warn!("swapping explicit txouts is unsupported");
                None
            }
        })
    })
    .await?;

    let (bobs_fee_rate, fee_offset) = if fee_asset == sell_asset {
        // Bob currently hardcodes a fee-rate of 1 sat / vbyte, hence
        // there is no need for us to perform fee estimation. Later
        // on, both parties should probably agree on a block-target
        // and use the same estimation service.
        let bobs_fee_rate = Amount::from_sat(1);
        let fee_offset = calculate_fee_offset(bobs_fee_rate);

        (bobs_fee_rate, fee_offset)
    } else {
        (Amount::ZERO, Amount::ZERO)
    };

    let output = coin_select(
        utxos,
        sell_amount,
        bobs_fee_rate.as_sat() as f32,
        fee_offset,
    )
    .context("failed to select coins")?;

    Ok(CreateSwapPayload {
        address: wallet.get_address(),
        alice_inputs: output
            .coins
            .into_iter()
            .map(|utxo| SwapUtxo {
                outpoint: utxo.outpoint,
                blinding_key,
            })
            .collect(),
        amount: output.target_amount,
    })
}

/// Calculate the fee offset required for the coin selection algorithm.
///
/// We are calculating this fee offset here so that we select enough coins to pay for the asset + the fee.
fn calculate_fee_offset(fee_sats_per_vbyte: Amount) -> Amount {
    let bobs_outputs = 2; // bob will create two outputs for himself (receive + change)
    let our_output = 1; // we have one additional output (the change output is priced in by the coin-selection algorithm)

    let fee_offset =
        ((bobs_outputs + our_output) * avg_vbytes::OUTPUT) * fee_sats_per_vbyte.as_sat();

    Amount::from_sat(fee_offset)
}
