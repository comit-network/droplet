import { ExternalLinkIcon } from "@chakra-ui/icons";
import { Box, Button, Center, Flex, Image, Link, Text, VStack } from "@chakra-ui/react";
import Debug from "debug";
import React, { useEffect, useReducer, useState } from "react";
import { useAsync } from "react-async";
import { useSSE } from "react-hooks-sse";
import { Route, Switch, useHistory, useParams } from "react-router-dom";
import useSWR from "swr";
import "./App.css";
import { postBuyPayload, postSellPayload } from "./Bobtimus";
import calculateBetaAmount, { getDirection } from "./calculateBetaAmount";
import AssetSelector from "./components/AssetSelector";
import COMIT from "./components/comit_logo_spellout_opacity_50.svg";
import ExchangeIcon from "./components/ExchangeIcon";
import WalletBalances from "./WalletBalances";
import {
    extractTrade,
    getBalances,
    getWalletStatus,
    makeBuyCreateSwapPayload,
    makeSellCreateSwapPayload,
    Trade,
} from "./wasmProxy";

const debug = Debug("App");

export enum Asset {
    LBTC = "L-BTC",
    USDT = "USDt",
}

export type AssetSide = "Alpha" | "Beta";

export type Action =
    | { type: "UpdateAlphaAmount"; value: string }
    | { type: "UpdateAlphaAssetType"; value: Asset }
    | { type: "UpdateBetaAssetType"; value: Asset }
    | {
        type: "SwapAssetTypes";
        value: {
            betaAmount: number;
        };
    }
    | { type: "PublishTransaction"; value: string }
    | { type: "UpdateWalletStatus"; value: WalletStatus }
    | { type: "UpdateBalance"; value: Balances };

interface State {
    alpha: AssetState;
    beta: Asset;
    txId: string;
    wallet: Wallet;
}

export interface Rate {
    ask: number;
    bid: number;
}

interface Wallet {
    status: WalletStatus;
}

interface WalletStatus {
    exists: boolean;
    loaded: boolean;
}

export interface Balances {
    usdt: number;
    btc: number;
}

interface AssetState {
    type: Asset;
    amount: string;
}

const initialState = {
    alpha: {
        type: Asset.LBTC,
        amount: "0.01",
    },
    beta: Asset.USDT,
    rate: {
        ask: 33766.30,
        bid: 33670.10,
    },
    txId: "",
    wallet: {
        balance: {
            usdtBalance: 0,
            btcBalance: 0,
        },
        status: {
            exists: false,
            loaded: false,
        },
    },
};

export function reducer(state: State = initialState, action: Action) {
    switch (action.type) {
        case "UpdateAlphaAmount":
            return {
                ...state,
                alpha: {
                    type: state.alpha.type,
                    amount: action.value,
                },
            };
        case "UpdateAlphaAssetType":
            let beta = state.beta;
            if (beta === action.value) {
                beta = state.alpha.type;
            }
            return {
                ...state,
                beta,
                alpha: {
                    type: action.value,
                    amount: state.alpha.amount,
                },
            };

        case "UpdateBetaAssetType":
            let alpha = state.alpha;
            if (alpha.type === action.value) {
                alpha.type = state.beta;
            }
            return {
                ...state,
                alpha,
                beta: action.value,
            };
        case "SwapAssetTypes":
            return {
                ...state,
                alpha: {
                    type: state.beta,
                    amount: state.alpha.amount,
                },
                beta: state.alpha.type,
            };
        case "PublishTransaction":
            return {
                ...state,
            };
        case "UpdateBalance":
            return {
                ...state,
                wallet: {
                    ...state.wallet,
                    balance: {
                        usdtBalance: action.value.usdt,
                        btcBalance: action.value.btc,
                    },
                },
            };
        case "UpdateWalletStatus":
            return {
                ...state,
                wallet: {
                    ...state.wallet,
                    status: {
                        exists: action.value.exists,
                        loaded: action.value.loaded,
                    },
                },
            };
        default:
            throw new Error("Unknown update action received");
    }
}

function App() {
    const history = useHistory();
    const path = history.location.pathname;

    useEffect(() => {
        if (path === "/app") {
            history.replace("/");
        }
    }, [path, history]);

    const [[transaction, trade], setTransaction] = useState<[string, Trade]>(["", {} as any]);
    const [state, dispatch] = useReducer(reducer, initialState);

    const rate = useSSE("rate", {
        ask: 33766.30,
        bid: 33670.10,
    });

    // TODO window.wallet_status does not yet exist... need to get event listener
    let { data: getWalletStatusResponse, isLoading, reload: reloadWalletStatus } = useAsync({
        promiseFn: getWalletStatus,
    });

    useEffect(() => {
        let callback = (_message: MessageEvent) => {};
        // @ts-ignore
        if (!window.wallet_status) {
            callback = (message: MessageEvent) => {
                debug("Received message: %s", message.data);
                reloadWalletStatus();
            };
        }
        window.addEventListener("message", callback);

        return () => window.removeEventListener("message", callback);
    });

    let walletStatus = getWalletStatusResponse || { exists: false, loaded: false };

    let { data: getBalancesResponse, mutate: reloadWalletBalances } = useSWR(
        () => walletStatus.loaded ? "wallet-balances" : null,
        () => getBalances(),
        {
            // TODO uncomment this and remove revalidateOnFocus, it just produces annoying log messages during development
            // refreshInterval: 5000,
            revalidateOnFocus: false,
        },
    );
    let balances = getBalancesResponse || [];

    let btcBalanceEntry = balances.find(
        balance => balance.ticker === Asset.LBTC,
    );
    let usdtBalanceEntry = balances.find(
        balance => balance.ticker === Asset.USDT,
    );

    const btcBalance = btcBalanceEntry ? btcBalanceEntry.value : 0;
    const usdtBalance = usdtBalanceEntry ? usdtBalanceEntry.value : 0;

    let { run: makeNewSwap, isLoading: isCreatingNewSwap } = useAsync({
        deferFn: async () => {
            let payload;
            let tx;
            if (state.alpha.type === Asset.LBTC) {
                payload = await makeSellCreateSwapPayload(state.alpha.amount.toString());
                tx = await postSellPayload(payload);
            } else {
                payload = await makeBuyCreateSwapPayload(state.alpha.amount.toString());
                tx = await postBuyPayload(payload);
            }

            let trade = await extractTrade(tx);

            setTransaction([tx, trade]);

            // TODO: call confirm swap through to BS
            // await confirmSwap();
        },
    });

    const alphaAmount = Number.parseFloat(state.alpha.amount);
    const betaAmount = calculateBetaAmount(
        state.alpha.type,
        alphaAmount,
        rate,
    );

    let walletBalances;

    async function unlock_wallet() {
        // TODO send request to open popup to unlock wallet
        // @ts-ignore
        debug("For now open popup manually...");
    }
    async function open_wallet_popup() {
        // TODO send request to open popup to show balances
        // @ts-ignore
        debug("For now open popup manually...");
    }

    if (!walletStatus.exists) {
        walletBalances = <Button
            onClick={async () => {
                await unlock_wallet();
            }}
            size="sm"
            variant="primary"
            isLoading={isLoading}
            data-cy="create-wallet-button"
        >
            Create wallet
        </Button>;
    } else if (walletStatus.exists && !walletStatus.loaded) {
        walletBalances = <Button
            onClick={unlock_wallet}
            size="sm"
            variant="primary"
            isLoading={isLoading}
            data-cy="unlock-wallet-button"
        >
            Unlock wallet
        </Button>;
    } else {
        walletBalances = <WalletBalances
            balances={{
                usdt: usdtBalance,
                btc: btcBalance,
            }}
            onClick={open_wallet_popup}
        />;
    }

    let isSwapButtonDisabled = state.alpha.type === Asset.LBTC
        ? btcBalance < alphaAmount
        : usdtBalance < alphaAmount;

    return (
        <Box className="App">
            <header className="App-header">
                {walletBalances}
            </header>
            <Center className="App-body">
                <Switch>
                    <Route exact path="/">
                        <VStack spacing={4} align="stretch">
                            <Flex color="white">
                                <AssetSelector
                                    assetSide="Alpha"
                                    placement="left"
                                    amount={state.alpha.amount}
                                    type={state.alpha.type}
                                    dispatch={dispatch}
                                />
                                <Center w="10px">
                                    <Box zIndex={2}>
                                        <ExchangeIcon
                                            onClick={() =>
                                                dispatch({
                                                    type: "SwapAssetTypes",
                                                    value: {
                                                        betaAmount,
                                                    },
                                                })}
                                            dataCy="exchange-asset-types-button"
                                        />
                                    </Box>
                                </Center>
                                <AssetSelector
                                    assetSide="Beta"
                                    placement="right"
                                    amount={betaAmount}
                                    type={state.beta}
                                    dispatch={dispatch}
                                />
                            </Flex>
                            <RateInfo rate={rate} direction={getDirection(state.alpha.type)} />
                            <Box>
                                <Button
                                    onClick={makeNewSwap}
                                    variant="primary"
                                    w="15rem"
                                    isLoading={isCreatingNewSwap}
                                    disabled={isSwapButtonDisabled}
                                    data-cy="swap-button"
                                >
                                    Swap
                                </Button>
                            </Box>
                        </VStack>
                    </Route>

                    <Route exact path="/swapped/:txId">
                        <VStack>
                            <Text textStyle="smGray">
                                Check in{" "}
                                <BlockExplorerLink />
                            </Text>
                        </VStack>
                    </Route>
                </Switch>
            </Center>

            <footer className="App-footer">
                <Center>
                    <Image src={COMIT} h="24px" />
                </Center>
            </footer>
        </Box>
    );
}

function BlockExplorerLink() {
    const { txId } = useParams<{ txId: string }>();
    const baseUrl = process.env.REACT_APP_BLOCKEXPLORER_URL
        ? `${process.env.REACT_APP_BLOCKEXPLORER_URL}`
        : "https://blockstream.info/liquid";

    return <Link
        href={`${baseUrl}/tx/${txId}`}
        isExternal
    >
        Block Explorer <ExternalLinkIcon mx="2px" />
    </Link>;
}

interface RateInfoProps {
    rate: Rate;
    direction: "ask" | "bid";
}

function RateInfo({ rate, direction }: RateInfoProps) {
    switch (direction) {
        case "ask":
            return <Box>
                <Text textStyle="smGray">{rate.ask} USDT ~ 1 BTC</Text>
            </Box>;
        case "bid":
            return <Box>
                <Text textStyle="smGray">1 BTC ~ {rate.bid} USDT</Text>
            </Box>;
    }
}

export default App;
