import { act, render, screen } from "@testing-library/react";
import React from "react";
import { Listener, Source, SSEProvider } from "react-hooks-sse";
import { BrowserRouter } from "react-router-dom";
import App, { Asset, reducer } from "./App";
import calculateBetaAmount from "./calculateBetaAmount";

const defaultState = {
    alpha: {
        type: Asset.LBTC,
        amount: "0",
    },
    beta: Asset.USDT,
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

test("update alpha amount logic", () => {
    const initialState = {
        ...defaultState,
        alpha: {
            type: Asset.LBTC,
            amount: "0.01",
        },
        beta: Asset.USDT,
    };

    let newValue = "42";
    expect(
        reducer(initialState, {
            type: "UpdateAlphaAmount",
            value: newValue,
        }).alpha.amount,
    ).toBe(newValue);
});

test("update alpha asset logic - should flip asset types", () => {
    const initialState = {
        ...defaultState,
        alpha: {
            type: Asset.LBTC,
            amount: "0.01",
        },
        beta: Asset.USDT,
    };

    let newValue = Asset.USDT;
    expect(
        reducer(initialState, {
            type: "UpdateAlphaAssetType",
            value: newValue,
        }).alpha.type,
    ).toBe(newValue);

    expect(
        reducer(initialState, {
            type: "UpdateAlphaAssetType",
            value: newValue,
        }).beta,
    ).toBe(initialState.alpha.type);

    // amounts should be unchanged
    expect(
        reducer(initialState, {
            type: "UpdateAlphaAssetType",
            value: newValue,
        }).alpha.amount,
    ).toBe(initialState.alpha.amount);
});

test("update beta asset logic - should flip asset types", () => {
    const initialState = {
        ...defaultState,
        alpha: {
            type: Asset.LBTC,
            amount: "0.01",
        },
        beta: Asset.USDT,
    };

    let newValue = Asset.LBTC;
    expect(
        reducer(initialState, {
            type: "UpdateBetaAssetType",
            value: newValue,
        }).beta,
    ).toBe(newValue);

    expect(
        reducer(initialState, {
            type: "UpdateBetaAssetType",
            value: newValue,
        }).alpha.type,
    ).toBe(initialState.beta);

    // amounts should be unchanged
    expect(
        reducer(initialState, {
            type: "UpdateBetaAssetType",
            value: newValue,
        }).alpha.amount,
    ).toBe(initialState.alpha.amount);
});

test("Swap asset types", () => {
    const initialState = {
        ...defaultState,
        alpha: {
            type: Asset.LBTC,
            amount: "0.01",
        },
        beta: Asset.USDT,
    };

    const rate = {
        bid: 10,
        ask: 10,
    };
    // This is just showing how the beta amount is calculated in "reality". The actual amounts and rates don't matter
    // in this test.
    let amountAsNumber = Number.parseFloat(initialState.alpha.amount);
    let betaAmount = calculateBetaAmount(initialState.alpha.type, amountAsNumber, rate);

    expect(
        reducer(initialState, {
            type: "SwapAssetTypes",
            value: {
                betaAmount: betaAmount,
            },
        }).alpha.type,
    ).toBe(initialState.beta);

    expect(
        reducer(initialState, {
            type: "SwapAssetTypes",
            value: {
                betaAmount: betaAmount,
            },
        }).beta,
    ).toBe(initialState.alpha.type);

    // amounts should not be flipped.
    expect(
        reducer(initialState, {
            type: "SwapAssetTypes",
            value: {
                betaAmount: betaAmount,
            },
        }).alpha.amount,
    ).toBe(
        initialState.alpha.amount.toString(),
    );
});
