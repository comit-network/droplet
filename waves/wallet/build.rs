use anyhow::{bail, Context, Result};
use elements_fun::AssetId;
use std::{env, fs, path::Path};

fn main() -> Result<()> {
    let out_dir = env::var_os("OUT_DIR").context("unable to access OUT_DIR")?;
    let constants_rs = Path::new(&out_dir).join("constants.rs");

    let native_asset_ticker = option_env!("NATIVE_ASSET_TICKER").unwrap_or("L-BTC");

    let native_asset_id = option_env!("NATIVE_ASSET_ID")
        .unwrap_or("6f0279e9ed041c3d710a9f57d0c02928416460c4b722ae3457a11eec381c526d");
    let native_asset_id = native_asset_id
        .parse::<AssetId>()
        .with_context(|| format!("failed to parse {} as asset id", native_asset_id))?;

    let usdt_asset_id = option_env!("USDT_ASSET_ID")
        .unwrap_or("ce091c998b83c78bb71a632313ba3760f1763d9cfcffae02258ffa9865a37bd2");
    let usdt_asset_id = usdt_asset_id
        .parse::<AssetId>()
        .with_context(|| format!("failed to parse {} as asset id", usdt_asset_id))?;

    let esplora_api_url = option_env!("ESPLORA_API_URL")
        .as_deref()
        .unwrap_or("https://blockstream.info/liquid/api");

    let address_params = match option_env!("CHAIN") {
        None | Some("LIQUID") => "&elements_fun::AddressParams::LIQUID",
        Some("ELEMENTS") => "&elements_fun::AddressParams::ELEMENTS",
        Some(chain) => bail!("unsupported elements chain {}", chain),
    };

    let rate = option_env!("DEFAULT_SAT_PER_VBYTE")
        .as_deref()
        .unwrap_or("1.0");
    let rate = rate
        .parse::<f64>()
        .with_context(|| format!("failed to parse '{}' as f64", rate))?;

    fs::write(
        &constants_rs,
        &format!(
            r#"
pub const NATIVE_ASSET_TICKER: &str = "{}";
pub const NATIVE_ASSET_ID: elements_fun::AssetId = elements_fun::AssetId::from_bytes({:?});
pub const USDT_ASSET_ID: elements_fun::AssetId = elements_fun::AssetId::from_bytes({:?});
pub const ESPLORA_API_URL: &str = "{}";
pub const ADDRESS_PARAMS: &elements_fun::AddressParams = {};
pub const DEFAULT_SAT_PER_VBYTE: f32 = {:.4};
"#,
            native_asset_ticker,
            native_asset_id.into_bytes(),
            usdt_asset_id.into_bytes(),
            esplora_api_url,
            address_params,
            rate
        ),
    )
    .context("failed to write constants.rs file")?;

    Ok(())
}
