use crate::jupiter::JupiterProgramIx;
use crate::orca::OrcaProgramIx;
use crate::raydium::RaydiumProgramIx;
use crate::raydium_amm::RaydiumAmmProgramIx;
use crate::token_extension_program::TokenExtensionProgramIx;
use crate::token_program::TokenProgramIx;
use solana_program::{pubkey, pubkey::Pubkey};
use yellowstone_vixen_core::{instruction::InstructionUpdate, Parser};

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
);

pub trait InstructionParser: Send + Sync + std::fmt::Debug {
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
    T: Parser<Input = InstructionUpdate> + Send + Sync + std::fmt::Debug,
    T::Output: Into<TransactionInstruction>,
{
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
        T: Parser<Input = InstructionUpdate> + Send + Sync + std::fmt::Debug + 'static,
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
    pub instructions: Vec<Box<TransactionInstruction>>,
}

pub const ID: Pubkey = pubkey!("675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8");

impl yellowstone_vixen_core::Parser for TransactionParser {
    type Input = yellowstone_vixen_core::TransactionUpdate;
    type Output = TransactionOutput;

    fn id(&self) -> std::borrow::Cow<str> {
        "Vixen::TransactionParser".into()
    }

    fn prefilter(&self) -> yellowstone_vixen_core::Prefilter {
        yellowstone_vixen_core::Prefilter::builder()
            .transaction_accounts([ID])
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
        Ok(TransactionOutput {
            slot: tx_update.slot,
            signature: tx_update.transaction.as_ref().unwrap().signature.clone(),
            instructions,
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
    use yellowstone_vixen_core::proto::ParseProto;
    use yellowstone_vixen_proto::parser::TransactionOutputProto;

    use super::{TransactionInstruction, TransactionOutput, TransactionParser};
    use crate::helpers::IntoProto;

    impl IntoProto<TransactionOutputProto> for TransactionOutput {
        fn into_proto(self) -> TransactionOutputProto {
            TransactionOutputProto {
                slot: self.slot,
                signature: bs58::encode(&self.signature).into_string(),
                instructions: self
                    .instructions
                    .into_iter()
                    .map(IntoProto::into_proto)
                    .collect(),
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
