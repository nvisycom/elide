//! Content streams as operations: read through hayro-syntax, each operand
//! owned as a [`Value`], and written back through pdf-writer.

use elide_core::Result;
use hayro_syntax::content::UntypedIter;
use pdf_writer::Content;

use crate::pdf::object::Value;
use crate::pdf::writer::{object_value, write_value};

/// One operator of a content stream and the operands before it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Operation {
    pub(crate) operator: String,
    pub(crate) operands: Vec<Value>,
}

impl Operation {
    /// The operation `operator` on `operands`.
    pub(crate) fn new(operator: &str, operands: Vec<Value>) -> Self {
        Self {
            operator: operator.to_owned(),
            operands,
        }
    }
}

/// The operations of the content `bytes`, up to the first it cannot read.
/// An inline image (`BI … EI`) is one `BI` operation whose operand is its
/// image, a stream.
pub(crate) fn parse(bytes: &[u8]) -> Vec<Operation> {
    let mut operations = Vec::new();
    let mut iter = UntypedIter::new(bytes);
    while let Some(instruction) = iter.next() {
        operations.push(Operation {
            operator: String::from_utf8_lossy(instruction.operator).into_owned(),
            operands: instruction.operands().map(object_value).collect(),
        });
    }
    operations
}

/// `operations` written as content.
///
/// # Errors
///
/// As [`write_value`]: an operand that is a stream (an inline image, which
/// is not written) or a number that is not finite.
pub(crate) fn encode(operations: &[Operation]) -> Result<Vec<u8>> {
    let mut content = Content::new();
    for operation in operations {
        let mut op = content.op(&operation.operator);
        for operand in &operation.operands {
            write_value(op.obj(), operand)?;
        }
    }
    Ok(content.finish().to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Content read and written back reads as the same operations.
    #[test]
    fn content_round_trips() {
        let bytes = b"q 1 0 0 1 72.5 -3 cm BT /F1 12 Tf [(Hi) -250 <0041>] TJ ET \
                      /OC /L1 BDC EMC /P << /MCID 0 >> BDC EMC Q";
        let operations = parse(bytes);
        assert_eq!(operations.len(), 11);
        assert_eq!(operations[1].operands[4], Value::Real(72.5));
        assert_eq!(operations[1].operands[5], Value::Int(-3));
        assert_eq!(parse(&encode(&operations).unwrap()), operations);
    }
}
