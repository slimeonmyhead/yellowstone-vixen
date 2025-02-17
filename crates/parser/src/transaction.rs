use crate::jupiter::JupiterProgramIx;
use crate::orca::OrcaProgramIx;
use crate::pumpfun::PumpfunProgramIx;
use crate::raydium::RaydiumProgramIx;
use crate::raydium_amm::RaydiumAmmProgramIx;
use crate::token_extension_program::TokenExtensionProgramIx;
use crate::token_program::TokenProgramIx;
use solana_program::{pubkey, pubkey::Pubkey};
use yellowstone_grpc_proto::prelude::TokenBalance;
use yellowstone_vixen_core::{instruction::InstructionUpdate, Parser, ProgramParser};

#[cfg(feature = "proto")]
use yellowstone_vixen_proto::parser::{instruction_proto::IxOneof, InstructionProto};

macro_rules! define_instruction_variants {
    ($(($variant:ident, $type:ty, $proto_variant:ident)),* $(,)?) => {
        #[derive(Debug)]
        pub enum TransactionInstruction {
            $($variant($type),)*
        }

        $(
            impl From<$type> for TransactionInstruction {
                fn from(ix: $type) -> Self {
                    TransactionInstruction::$variant(ix)
                }
            }
        )*

        #[cfg(feature = "proto")]
        impl crate::helpers::IntoProto<InstructionProto> for TransactionInstruction {
            fn into_proto(self) -> InstructionProto {
                match self {
                    $(
                        TransactionInstruction::$variant(ix) => InstructionProto {
                            ix_oneof: Some(IxOneof::$proto_variant(ix.into_proto())),
                        },
                    )*
                }
            }
        }
    };
}

define_instruction_variants!(
    (Jupiter, JupiterProgramIx, JupiterProgramIx),
    (Raydium, RaydiumProgramIx, RaydiumProgramIx),
    (Token, TokenProgramIx, TokenProgramIx),
    (
        TokenExtension,
        TokenExtensionProgramIx,
        TokenExtensionProgramIx
    ),
    (Orca, OrcaProgramIx, OrcaProgramIx),
    (RaydiumAmm, RaydiumAmmProgramIx, RaydiumAmmProgramIx),
    (Pumpfun, PumpfunProgramIx, PumpfunProgramIx),
);

pub trait InstructionParser: Send + Sync + std::fmt::Debug {
    fn program_id(&self) -> yellowstone_vixen_core::Pubkey;

    fn parse_instruction<'a>(
        &'a self,
        input: &'a InstructionUpdate,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = yellowstone_vixen_core::ParseResult<TransactionInstruction>,
                > + Send
                + 'a,
        >,
    >;
}

impl<T> InstructionParser for T
where
    T: Parser<Input = InstructionUpdate> + ProgramParser + Send + Sync + std::fmt::Debug,
    T::Output: Into<TransactionInstruction>,
{
    fn program_id(&self) -> yellowstone_vixen_core::Pubkey {
        ProgramParser::program_id(self)
    }

    fn parse_instruction<'a>(
        &'a self,
        input: &'a InstructionUpdate,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = yellowstone_vixen_core::ParseResult<TransactionInstruction>,
                > + Send
                + 'a,
        >,
    > {
        Box::pin(async move { self.parse(input).await.map(|output| output.into()) })
    }
}

#[derive(Debug)]
pub struct TransactionParser {
    pub instruction_parsers: Vec<Box<dyn InstructionParser>>,
}

impl TransactionParser {
    pub fn builder() -> TransactionParserBuilder {
        TransactionParserBuilder::new()
    }
}

#[derive(Debug, Default)]
pub struct TransactionParserBuilder {
    instruction_parsers: Vec<Box<dyn InstructionParser>>,
}

impl TransactionParserBuilder {
    pub fn new() -> Self {
        Self {
            instruction_parsers: Vec::new(),
        }
    }

    pub fn instruction<T>(mut self, parser: T) -> Self
    where
        T: Parser<Input = InstructionUpdate>
            + ProgramParser
            + Send
            + Sync
            + std::fmt::Debug
            + 'static,
        T::Output: Into<TransactionInstruction>,
    {
        self.instruction_parsers.push(Box::new(parser));
        self
    }

    pub fn build(self) -> TransactionParser {
        TransactionParser {
            instruction_parsers: self.instruction_parsers,
        }
    }
}

#[derive(Debug)]
pub struct TransactionOutput {
    pub slot: u64,
    pub signature: Vec<u8>,
    pub signer: Pubkey,
    pub timestamp: i64,
    pub instructions: Vec<Box<TransactionInstruction>>,
    pub accounts: Vec<Pubkey>,
    pub pre_balances: Vec<u64>,
    pub post_balances: Vec<u64>,
    pub pre_token_balances: Vec<TokenBalance>,
    pub post_token_balances: Vec<TokenBalance>,
}

pub const EXCLUDED_PROGRAM_IDS: &[Pubkey] = &[
    pubkey!("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA"),
    pubkey!("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb"),
];
pub const ID: Pubkey = pubkey!("Transaction11111111111111111111111111111111");

impl yellowstone_vixen_core::Parser for TransactionParser {
    type Input = yellowstone_vixen_core::TransactionUpdate;
    type Output = TransactionOutput;

    fn id(&self) -> std::borrow::Cow<str> {
        "Vixen::TransactionParser".into()
    }

    fn prefilter(&self) -> yellowstone_vixen_core::Prefilter {
        let program_ids: Vec<yellowstone_vixen_core::Pubkey> = self
            .instruction_parsers
            .iter()
            .map(|parser| parser.program_id())
            .filter(|id| !EXCLUDED_PROGRAM_IDS.contains(&id.into_bytes().into()))
            .collect();

        yellowstone_vixen_core::Prefilter::builder()
            .transaction_accounts(program_ids)
            .build()
            .unwrap()
    }

    async fn parse(
        &self,
        tx_update: &yellowstone_vixen_core::TransactionUpdate,
    ) -> yellowstone_vixen_core::ParseResult<Self::Output> {
        let ixs = InstructionUpdate::parse_from_txn(tx_update).unwrap();
        let mut instructions: Vec<Box<TransactionInstruction>> = Vec::new();
        for insn in ixs.iter().flat_map(|i| i.visit_all()) {
            for parser in &self.instruction_parsers {
                if let Ok(parsed) = parser.parse_instruction(insn).await {
                    instructions.push(Box::new(parsed));
                }
            }
        }

        let message = tx_update
            .transaction
            .as_ref()
            .unwrap()
            .transaction
            .as_ref()
            .unwrap()
            .message
            .as_ref()
            .unwrap();

        let accounts: Vec<Pubkey> = message
            .account_keys
            .clone()
            .into_iter()
            .map(|a| Pubkey::new_from_array(a.as_slice().try_into().unwrap_or([0; 32])))
            .collect();

        let meta = tx_update
            .transaction
            .as_ref()
            .unwrap()
            .meta
            .as_ref()
            .unwrap();

        Ok(TransactionOutput {
            slot: tx_update.slot,
            signature: tx_update.transaction.as_ref().unwrap().signature.clone(),
            signer: *accounts.get(0).unwrap_or(&Pubkey::new_from_array([0; 32])),
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs() as i64,
            instructions,
            accounts: accounts.clone(),
            pre_balances: meta.post_balances.clone(),
            post_balances: meta.pre_balances.clone(),
            pre_token_balances: meta.pre_token_balances.clone(),
            post_token_balances: meta.post_token_balances.clone(),
        })
    }
}

impl yellowstone_vixen_core::ProgramParser for TransactionParser {
    #[inline]
    fn program_id(&self) -> yellowstone_vixen_core::Pubkey {
        ID.to_bytes().into()
    }
}

#[cfg(feature = "proto")]
mod proto_parser {
    use yellowstone_grpc_proto::prelude::{TokenBalance, UiTokenAmount};
    use yellowstone_vixen_core::proto::ParseProto;
    use yellowstone_vixen_proto::parser::{
        TokenBalanceProto, TransactionOutputProto, UiTokenAmountProto,
    };

    use super::{TransactionInstruction, TransactionOutput, TransactionParser};
    use crate::helpers::IntoProto;

    impl IntoProto<TransactionOutputProto> for TransactionOutput {
        fn into_proto(self) -> TransactionOutputProto {
            TransactionOutputProto {
                slot: self.slot,
                signature: bs58::encode(&self.signature).into_string(),
                signer: bs58::encode(&self.signer).into_string(),
                timestamp: self.timestamp,
                instructions: self
                    .instructions
                    .into_iter()
                    .map(IntoProto::into_proto)
                    .collect(),
                accounts: self.accounts.iter().map(|a| a.to_string()).collect(),
                pre_balances: self.pre_balances.to_vec(),
                post_balances: self.post_balances.to_vec(),
                pre_token_balances: self
                    .pre_token_balances
                    .into_iter()
                    .map(IntoProto::into_proto)
                    .collect(),
                post_token_balances: self
                    .post_token_balances
                    .into_iter()
                    .map(IntoProto::into_proto)
                    .collect(),
            }
        }
    }

    impl IntoProto<TokenBalanceProto> for TokenBalance {
        fn into_proto(self) -> TokenBalanceProto {
            TokenBalanceProto {
                account_index: self.account_index,
                mint: self.mint.to_string(),
                ui_token_amount: self.ui_token_amount.and_then(|amt| Some(amt.into_proto())),
                owner: self.owner.to_string(),
                program_id: self.program_id.to_string(),
            }
        }
    }

    impl IntoProto<UiTokenAmountProto> for UiTokenAmount {
        fn into_proto(self) -> UiTokenAmountProto {
            UiTokenAmountProto {
                ui_amount: self.ui_amount,
                decimals: self.decimals,
                amount: self.amount.to_string(),
                ui_amount_string: self.ui_amount_string,
            }
        }
    }

    impl ParseProto for TransactionParser {
        type Message = TransactionOutputProto;

        fn output_into_message(value: Self::Output) -> Self::Message {
            IntoProto::<TransactionOutputProto>::into_proto(value)
        }
    }

    impl IntoProto<yellowstone_vixen_proto::parser::InstructionProto> for Box<TransactionInstruction> {
        fn into_proto(self) -> yellowstone_vixen_proto::parser::InstructionProto {
            (*self).into_proto()
        }
    }
}
