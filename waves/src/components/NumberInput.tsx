import { InputGroup, InputLeftAddon, NumberInputField, NumberInputStepper } from "@chakra-ui/react";
import { NumberInput as CUINumberInput } from "@chakra-ui/react";
import React from "react";

type StringOrNumber = string | number;

function NumberInput({ currency, value, onAmountChange, precision, step, isDisabled, data_cy }: CustomInputProps) {
    const inputProps = isDisabled ? ASSET_INPUT_DISABLED_PROPS : ASSET_INPUT_PROPS;
    return (
        <InputGroup>
            <InputLeftAddon
                {...ASSET_INPUT_LEFT_ADDON_PROPS}
                children={currency}
            />
            <CUINumberInput
                {...inputProps}
                onChange={(valueString, _) => onAmountChange(valueString)}
                value={value}
                precision={precision}
                step={step}
                isDisabled={isDisabled}
                min={0}
                data-cy={`${data_cy}-amount-input`}
            >
                <NumberInputField />
                <NumberInputStepper />
            </CUINumberInput>
        </InputGroup>
    );
}

interface CustomInputProps {
    currency: string;
    value: StringOrNumber;
    precision: number;
    step: number;
    onAmountChange: (val: string) => void;
    isDisabled: boolean;
    data_cy: string;
}

const ASSET_INPUT_LEFT_ADDON_PROPS = {
    size: "lg",
    textStyle: "lgGray",
    w: "15%",
    h: "3rem",
    bg: "grey.50",
    borderRadius: "md",
    shadow: "md",
};

const ASSET_INPUT_PROPS = {
    w: "100%",
    size: "lg",
    bg: "#FFFFFF",
    textStyle: "lgGray",
    borderRadius: "md",
    shadow: "md",
};

const ASSET_INPUT_DISABLED_PROPS = {
    ...ASSET_INPUT_PROPS,
    bg: "grey.50",
};

export default NumberInput;
