use crate::error::TokenizerError;
use crate::ToolCall;
use sha2::{Digest, Sha256};
use tokenizers::Tokenizer;

pub(crate) fn verify_asset_sha256(
    encoder: &'static str,
    bytes: &[u8],
    expected: &'static str,
) -> Result<(), TokenizerError> {
    use subtle::ConstantTimeEq;

    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let actual_bytes: [u8; 32] = hasher.finalize().into();
    let actual_hex = hex::encode(actual_bytes);

    let expected_vec = match hex::decode(expected) {
        Ok(v) if v.len() == 32 => v,
        _ => {
            return Err(TokenizerError::AssetSignatureMismatch {
                encoder,
                expected,
                actual: format!("expected-const-malformed (got {actual_hex})"),
            });
        }
    };

    if actual_bytes.as_slice().ct_eq(&expected_vec).into() {
        Ok(())
    } else {
        Err(TokenizerError::AssetSignatureMismatch {
            encoder,
            expected,
            actual: actual_hex,
        })
    }
}

pub(crate) fn load_tokenizer(
    encoder: &'static str,
    bytes: &[u8],
) -> Result<Tokenizer, TokenizerError> {
    Tokenizer::from_bytes(bytes).map_err(|e| TokenizerError::AssetLoadFailed {
        encoder,
        message: format!("Tokenizer::from_bytes failed: {e}"),
    })
}

pub(crate) fn cross_check(
    encoder: &'static str,
    tokenizer: &Tokenizer,
    fixture: &'static str,
    expected: &[u32],
) -> Result<(), TokenizerError> {
    let enc =
        tokenizer
            .encode(fixture, false)
            .map_err(|e| TokenizerError::AssetSignatureMismatch {
                encoder,
                expected: "cross_check_fixture_vector",
                actual: format!("fixture-encode-error: {e}"),
            })?;
    let actual = enc.get_ids();
    if actual != expected {
        let expected_summary: String = expected
            .iter()
            .take(6)
            .map(|t| t.to_string())
            .collect::<Vec<_>>()
            .join(",");
        let actual_summary: String = actual
            .iter()
            .take(6)
            .map(|t| t.to_string())
            .collect::<Vec<_>>()
            .join(",");
        return Err(TokenizerError::AssetSignatureMismatch {
            encoder,
            expected: "cross_check_fixture_vector",
            actual: format!(
                "fixture-vector-mismatch: expected first 6 tokens=[{expected_summary}], got=[{actual_summary}]"
            ),
        });
    }
    Ok(())
}

pub(crate) fn encode_count(
    encoder: &'static str,
    tokenizer: &Tokenizer,
    text: &str,
) -> Result<usize, TokenizerError> {
    if text.is_empty() {
        return Ok(0);
    }
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        tokenizer.encode(text, false)
    }));
    match result {
        Ok(Ok(enc)) => Ok(enc.get_ids().len()),
        Ok(Err(e)) => Err(TokenizerError::EncoderInternal {
            kind: encoder,
            message: format!("tokenizers encode error: {e}"),
        }),
        Err(_) => Err(TokenizerError::EncoderInternal {
            kind: encoder,
            message: "tokenizers encode panicked on input".to_string(),
        }),
    }
}

pub(crate) fn tool_call_tokens(
    encoder: &'static str,
    tokenizer: &Tokenizer,
    tc: &ToolCall,
) -> Result<usize, TokenizerError> {
    const TOOL_CALL_OVERHEAD: usize = 1;
    Ok(TOOL_CALL_OVERHEAD
        + encode_count(encoder, tokenizer, &tc.name)?
        + encode_count(encoder, tokenizer, &tc.arguments_json)?)
}
