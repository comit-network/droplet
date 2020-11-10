use hex::encode;
use hmac::{Hmac, Mac, NewMac};
use rand::{thread_rng, Rng};
use sha2::Sha256;
use std::fmt;
use std::{collections::HashMap, env::var, thread::sleep, time::Duration};
use testcontainers::core::{Container, Docker, Image, Port, WaitForMessage};

#[derive(Debug)]
pub struct ElementsCore {
    tag: String,
    arguments: ElementsCoreImageArgs,
    ports: Option<Vec<Port>>,
}

impl ElementsCore {
    pub fn auth(&self) -> &RpcAuth {
        &self.arguments.rpc_auth
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Network {
    Mainnet,
    Testnet,
    Regtest,
}

#[derive(Debug, Clone, Copy)]
pub enum AddressType {
    Legacy,
    P2shSegwit,
    Bech32,
}

impl fmt::Display for AddressType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            AddressType::Legacy => "legacy",
            AddressType::P2shSegwit => "p2sh-segwit",
            AddressType::Bech32 => "bech32",
        })
    }
}

#[derive(Clone, Debug)]
pub struct RpcAuth {
    pub username: String,
    pub password: String,
    pub salt: String,
}

impl RpcAuth {
    pub fn username(&self) -> &str {
        &self.username
    }

    pub fn password(&self) -> &str {
        &self.password
    }

    pub fn new(username: String) -> Self {
        let salt = Self::generate_salt();
        let password = Self::generate_password();

        RpcAuth {
            username,
            password,
            salt,
        }
    }

    fn generate_salt() -> String {
        let mut buffer = [0u8; 16];
        thread_rng().fill(&mut buffer[..]);
        encode(buffer)
    }

    fn generate_password() -> String {
        let mut buffer = [0u8; 32];
        thread_rng().fill(&mut buffer[..]);

        encode(buffer)
    }

    fn encode_password(&self) -> String {
        let mut mac = Hmac::<Sha256>::new_varkey(self.salt.as_bytes()).unwrap();
        mac.update(self.password.as_bytes().as_ref());

        let result = mac.finalize().into_bytes();

        encode(result)
    }

    pub fn encode(&self) -> String {
        format!("{}:{}${}", self.username, self.salt, self.encode_password())
    }
}

#[derive(Debug, Clone)]
pub struct ElementsCoreImageArgs {
    pub server: bool,
    pub network: Network,
    pub print_to_console: bool,
    pub tx_index: bool,
    pub rpc_bind: String,
    pub rpc_allowip: String,
    pub rpc_auth: RpcAuth,
    pub accept_non_std_txn: Option<bool>,
    pub rest: bool,
    pub validatepegin: bool,
}

impl Default for ElementsCoreImageArgs {
    fn default() -> Self {
        ElementsCoreImageArgs {
            server: true,
            network: Network::Regtest,
            print_to_console: true,
            rpc_auth: RpcAuth::new(String::from("elements")),
            tx_index: true,
            rpc_bind: "0.0.0.0".to_string(), // This allows to bind on all ports
            rpc_allowip: "0.0.0.0/0".to_string(),
            accept_non_std_txn: Some(false),
            rest: true,
            validatepegin: false,
        }
    }
}

impl IntoIterator for ElementsCoreImageArgs {
    type Item = String;
    type IntoIter = ::std::vec::IntoIter<String>;

    fn into_iter(self) -> <Self as IntoIterator>::IntoIter {
        let mut args = vec!["elementsd".to_string()];

        args.push(format!("-rpcauth={}", self.rpc_auth.encode()));

        if self.server {
            args.push("-server".to_string())
        }

        match self.network {
            Network::Testnet => args.push("-testnet".to_string()),
            Network::Regtest => args.push("-regtest".to_string()),
            Network::Mainnet => {}
        }

        if self.tx_index {
            args.push("-txindex=1".to_string())
        }

        if !self.rpc_allowip.is_empty() {
            args.push(format!("-rpcallowip={}", self.rpc_allowip));
        }

        if !self.rpc_bind.is_empty() {
            args.push(format!("-rpcbind={}", self.rpc_bind));
        }

        if self.print_to_console {
            args.push("-printtoconsole".to_string())
        }

        if let Some(accept_non_std_txn) = self.accept_non_std_txn {
            if accept_non_std_txn {
                args.push("-acceptnonstdtxn=1".to_string());
            } else {
                args.push("-acceptnonstdtxn=0".to_string());
            }
        }

        if self.rest {
            args.push("-rest".to_string())
        }

        if self.validatepegin {
            args.push("-validatepegin=1".to_string())
        } else {
            args.push("-validatepegin=0".to_string())
        }

        args.push("-debug".into());

        args.into_iter()
    }
}

impl Image for ElementsCore {
    type Args = ElementsCoreImageArgs;
    type EnvVars = HashMap<String, String>;
    type Volumes = HashMap<String, String>;
    type EntryPoint = std::convert::Infallible;

    fn descriptor(&self) -> String {
        format!("coblox/elements:{}", self.tag)
    }

    fn wait_until_ready<D: Docker>(&self, container: &Container<'_, D, Self>) {
        container
            .logs()
            .stdout
            .wait_for_message("Flushed wallet.dat")
            .unwrap();

        let additional_sleep_period =
            var("ELEMENTD_ADDITIONAL_SLEEP_PERIOD").map(|value| value.parse());

        if let Ok(Ok(sleep_period)) = additional_sleep_period {
            let sleep_period = Duration::from_millis(sleep_period);

            log::trace!(
                "Waiting for an additional {:?} for container {}.",
                sleep_period,
                container.id()
            );

            sleep(sleep_period)
        }
    }

    fn args(&self) -> <Self as Image>::Args {
        self.arguments.clone()
    }

    fn volumes(&self) -> Self::Volumes {
        HashMap::new()
    }

    fn env_vars(&self) -> Self::EnvVars {
        HashMap::new()
    }

    fn ports(&self) -> Option<Vec<Port>> {
        self.ports.clone()
    }

    fn with_args(self, arguments: <Self as Image>::Args) -> Self {
        ElementsCore { arguments, ..self }
    }
}

impl Default for ElementsCore {
    fn default() -> Self {
        ElementsCore {
            tag: "0.18.1.9".into(),
            arguments: ElementsCoreImageArgs::default(),
            ports: None,
        }
    }
}

impl ElementsCore {
    pub fn with_tag(self, tag_str: &str) -> Self {
        ElementsCore {
            tag: tag_str.to_string(),
            ..self
        }
    }

    pub fn with_mapped_port<P: Into<Port>>(mut self, port: P) -> Self {
        let mut ports = self.ports.unwrap_or_default();
        ports.push(port.into());
        self.ports = Some(ports);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_rpc_auth_correctly() {
        let auth = RpcAuth {
            username: "elements".to_string(),
            password: "password".to_string(),
            salt: "cb77f0957de88ff388cf817ddbc7273".to_string(),
        };

        let rpc_auth = auth.encode();

        assert_eq!(rpc_auth, "elements:cb77f0957de88ff388cf817ddbc7273$9565c5c6ed9bb1f0f0f3207e04b8a36129e92c0569f97ed0293919de56aece06".to_string())
    }
}