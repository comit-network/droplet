use crate::{
    components::{CreateWallet, TradeInfo, UnlockWallet, WalletDetails},
    event_bus::{EventBus, Response},
    wallet_updater::WalletUpdater,
};
use js_sys::Promise;
use message_types::bs_ps::{BackgroundStatus, ToBackground, TransactionData, WalletStatus};
use serde::{Deserialize, Serialize};
use wallet::BalanceEntry;
use wasm_bindgen::prelude::*;
use wasm_bindgen_extension::browser;
use wasm_bindgen_futures::{spawn_local, JsFuture};
use yew::prelude::*;

// We do not support renaming the wallet for now
pub const WALLET_NAME: &str = "demo-wallet";

pub struct App {
    link: ComponentLink<Self>,
    state: State,
    _event_bus: Box<dyn Bridge<EventBus>>,
    _wallet_updater: WalletUpdater,
}

pub enum Msg {
    CreateWallet,
    UnlockWallet,
    BackgroundStatus(Box<BackgroundStatus>),
    BalanceUpdate(Vec<BalanceEntry>),
    SignAndSend { tx_hex: String, tab_id: u32 },
    Reject { tx_hex: String, tab_id: u32 },
}

#[derive(Serialize, Deserialize, Clone)]
pub struct State {
    wallet_name: String,
    wallet_password: String,
    wallet_status: WalletStatus,
    wallet_balances: Vec<BalanceEntry>,
    sign_tx: Option<TransactionData>,
}

impl Component for App {
    type Message = Msg;
    type Properties = ();

    fn create(_props: Self::Properties, link: ComponentLink<Self>) -> Self {
        log::debug!("PopupApp: creating...");

        let inner_link = link.clone();
        send_to_backend(
            ToBackground::BackgroundStatusRequest,
            Box::new(move |response| {
                if let Ok(response) = response {
                    if let Ok(msg) = response.into_serde() {
                        inner_link.send_message(Msg::BackgroundStatus(msg));
                    }
                }
            }),
        );

        let mut wallet_updater = WalletUpdater::new();
        wallet_updater.spawn();
        let callback = link.callback(|response| match response {
            Response::WalletBalanceUpdate(balances) => Msg::BalanceUpdate(balances),
            Response::BackgroundStatus(background_status) => {
                Msg::BackgroundStatus(Box::new(background_status))
            }
        });
        App {
            link,
            state: State {
                wallet_name: WALLET_NAME.to_string(),
                wallet_password: "".to_string(),
                wallet_status: WalletStatus::None,
                sign_tx: None,
                wallet_balances: vec![],
            },
            _event_bus: EventBus::bridge(callback),
            _wallet_updater: wallet_updater,
        }
    }

    fn update(&mut self, msg: Self::Message) -> ShouldRender {
        match msg {
            Msg::UnlockWallet => {
                let inner_link = self.link.clone();
                send_to_backend(
                    ToBackground::UnlockRequest(
                        self.state.wallet_name.clone(),
                        self.state.wallet_password.clone(),
                    ),
                    Box::new(move |response| {
                        if let Ok(response) = response {
                            if let Ok(status) = response.into_serde() {
                                inner_link.send_message(Msg::BackgroundStatus(status));
                            }
                        }
                    }),
                );
                false
            }
            Msg::CreateWallet => {
                let inner_link = self.link.clone();
                send_to_backend(
                    ToBackground::CreateWalletRequest(
                        self.state.wallet_name.clone(),
                        self.state.wallet_password.clone(),
                    ),
                    Box::new(move |response| {
                        if response.is_ok() {
                            inner_link.send_message(Msg::BackgroundStatus(Box::new(
                                BackgroundStatus::new(WalletStatus::NotLoaded, None),
                            )));
                        }
                    }),
                );
                false
            }
            Msg::BackgroundStatus(status) => {
                self.state.wallet_status = status.wallet;
                self.state.sign_tx = status.sign_tx;

                true
            }
            Msg::SignAndSend { tx_hex, tab_id } => {
                let inner_link = self.link.clone();
                send_to_backend(
                    ToBackground::SignRequest { tx_hex, tab_id },
                    Box::new(move |response| {
                        if let Ok(response) = response {
                            if let Ok(status) = response.into_serde() {
                                inner_link.send_message(Msg::BackgroundStatus(status));
                            }
                        }
                    }),
                );
                false
            }
            Msg::Reject { tx_hex, tab_id } => {
                let inner_link = self.link.clone();
                send_to_backend(
                    ToBackground::Reject { tx_hex, tab_id },
                    Box::new(move |response| {
                        if let Ok(response) = response {
                            if let Ok(status) = response.into_serde() {
                                inner_link.send_message(Msg::BackgroundStatus(status));
                            }
                        }
                    }),
                );
                false
            }
            Msg::BalanceUpdate(balances) => {
                self.state.wallet_balances = balances;
                true
            }
        }
    }

    fn change(&mut self, _props: Self::Properties) -> bool {
        true
    }

    fn view(&self) -> Html {
        let wallet_form = match self.state.clone() {
            State {
                wallet_status: WalletStatus::NotLoaded,
                ..
            } => {
                html! {
                    <UnlockWallet on_form_submit=self.link.callback(|_| Msg::UnlockWallet)></UnlockWallet>
                }
            }
            State {
                wallet_status: WalletStatus::None,
                ..
            } => {
                html! {
                    <CreateWallet on_form_submit=self.link.callback(|_| Msg::CreateWallet)></CreateWallet>
                }
            }
            State {
                wallet_status: WalletStatus::Loaded { address },
                sign_tx: None,
                wallet_balances,
                ..
            } => {
                html! {
                    <WalletDetails address=address balances=wallet_balances></WalletDetails>
                }
            }
            State {
                wallet_status: WalletStatus::Loaded { .. },
                sign_tx:
                    Some(TransactionData {
                        hex,
                        decoded,
                        tab_id,
                    }),
                ..
            } => {
                let tx_hex = hex.clone();
                let sign_and_send = move |_| Msg::SignAndSend {
                    tx_hex: tx_hex.clone(),
                    tab_id,
                };
                let reject = move |_| Msg::Reject {
                    tx_hex: hex.clone(),
                    tab_id,
                };
                html! {

                    <>
                        <TradeInfo
                            trade=decoded
                            on_confirm=self.link.callback(sign_and_send)
                            on_reject=self.link.callback(reject)
                        >
                        </TradeInfo>
                    </>
                }
            }
        };

        let faucet_button = match &self.state.wallet_status {
            WalletStatus::Loaded { address, .. } => {
                let address = address.clone();
                html! {
                    <>
                        <ybc::Button
                            onclick=self.link.batch_callback(
                            move |_| {
                                faucet(address.to_string());
                                vec![]
                            })
                            classes="is-primary is-light">{ "Faucet" }
                        </ybc::Button>
                    </>
                }
            }
            _ => html! {},
        };

        html! {
            <ybc::Section>
                <ybc::Container>
                    <ybc::Box>
                        { wallet_form }
                    </ybc::Box>
                    // TODO: Feature flag this
                    {faucet_button}
                </ybc::Container>
            </ybc::Section>
        }
    }

    fn rendered(&mut self, _first_render: bool) {}

    fn destroy(&mut self) {}
}

fn faucet(address: String) {
    spawn_local(async move {
        let client = reqwest::Client::new();
        match client
            .post(format!("http://127.0.0.1:3030/api/faucet/{}", address).as_str())
            .send()
            .await
        {
            Ok(_) => {}
            Err(e) => log::error!("Call to faucet failed: {:?}", e),
        };
    })
}

fn send_to_backend(msg: ToBackground, callback: Box<dyn Fn(Result<JsValue, JsValue>)>) {
    spawn_local(async move {
        let js_value = JsValue::from_serde(&msg).unwrap();
        let promise: Promise = browser.runtime().send_message(None, &js_value, None);
        let result = JsFuture::from(promise).await;
        callback(result)
    });
}
