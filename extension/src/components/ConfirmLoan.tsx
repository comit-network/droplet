import { Box, Button, Flex, Heading, Image, Spacer, Text } from "@chakra-ui/react";
import React from "react";
import { useAsync } from "react-async";
import { signLoan } from "../background-proxy";
import { LoanToSign, USDT_TICKER } from "../models";
import YouSwapItem from "./SwapItem";
import Usdt from "./tether.svg";

interface ConfirmLoanProps {
    onCancel: (tabId: number) => void;
    onSuccess: () => void;
    loanToSign: LoanToSign;
}

export default function ConfirmLoan(
    { onCancel, onSuccess, loanToSign }: ConfirmLoanProps,
) {
    let { isPending, run } = useAsync({
        deferFn: async () => {
            await signLoan(loanToSign.tabId);
            onSuccess();
        },
    });

    let { details: { collateral, principal, principalRepayment, term } } = loanToSign;

    return (<Box>
        <form
            onSubmit={async e => {
                e.preventDefault();
                run();
            }}
            data-cy="confirm-loan-form"
        >
            <Heading>Confirm Loan</Heading>
            <Box>
                <YouSwapItem
                    tradeSide={collateral}
                    action={"send"}
                />
            </Box>
            <Box>
                <YouSwapItem
                    tradeSide={principal}
                    action={"receive"}
                />
            </Box>
            <Box w="100%">
                <Flex>
                    <Box h="40px" p="1">
                        <Text>Repayment amount: {principalRepayment}</Text>
                    </Box>
                    <Spacer />
                    <Box w="40px" h="40px">
                        <Image src={Usdt} h="32px" />
                    </Box>
                    <Box h="40px" justify="right" p="1">
                        <Text align="center" justify="center">
                            {USDT_TICKER}
                        </Text>
                    </Box>
                </Flex>
            </Box>
            <Box w="100%">
                <Flex>
                    <Box h="40px" p="1">
                        <Text>Loan term: {term}</Text>
                    </Box>
                </Flex>
            </Box>

            <Button
                variant="secondary"
                mr={3}
                onClick={() => onCancel(loanToSign.tabId)}
            >
                Cancel
            </Button>
            <Button
                type="submit"
                variant="primary"
                isLoading={isPending}
                data-cy="data-cy-sign-loan-button"
            >
                Sign
            </Button>
        </form>
    </Box>);
}
