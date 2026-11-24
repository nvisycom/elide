//! The redaction operators: what a [`Rule`](crate::rule::Rule) does to a matched
//! entity.
//!
//! Built with the [`Operator`] static constructors ([`Operator::replace`],
//! [`Operator::mask`], …) and folded into a rule with
//! [`Rule::label`](crate::rule::Rule::label) or
//! [`Rule::fallback`](crate::rule::Rule::fallback). One class wraps every
//! operator kind; which modality a stage's anonymizer accepts is an operator
//! question — text substitution applies to text and tabular, a cover to every
//! modality — checked when the anonymizer is built
//! (and, in TypeScript, branded so the mismatch is a compile-time error).

use elide::entity::LabelRef;
use elide::modality::audio::{Audio, Waveform as RustWaveform};
use elide::modality::image::Image;
use elide::modality::tabular::Tabular;
use elide::modality::text::Text;
use elide::modality::{Modality, TextRedactable};
use elide::primitive::LanguageTag;
use elide::redaction::operators::{
    AesEncrypt, Beep, Blur, Clamp, Cover, DateGranularity as RustDateGranularity,
    DateStyle as RustDateStyle, DropColumn, DropRow, Fake, GeneralizeDate, HmacHash, Keep, Mask,
    Pixelate, Remove, Replace, Sha2Algorithm as RustSha2Algorithm, Sha2Hash, Truncate,
};
use elide::redaction::{Operator as RustOperator, Rule as RustRule};
use serde::Deserialize;
use tsify::{Ts, Tsify};
use wasm_bindgen::prelude::*;

use crate::error::{ElideError, from_js, from_js_or_default, language_tag};

/// The key length [`Operator::encrypt`] takes: AES-256 keys are 32 bytes.
const AES_KEY_LEN: usize = 32;

/// The tuning of an [`Operator::mask`] operator: what it keeps and masks with.
///
/// `Tsify` generates the matching TypeScript `interface` (camelCase).
#[derive(Default, Deserialize, Tsify)]
#[serde(default, rename_all = "camelCase")]
pub struct MaskConfig {
    /// Leading characters to keep verbatim (default `0`).
    #[tsify(optional)]
    pub keep_prefix: usize,
    /// Trailing characters to keep verbatim (default `0`).
    #[tsify(optional)]
    pub keep_suffix: usize,
    /// The character each masked position is replaced with (default `*`). Only
    /// the first character is used.
    #[tsify(optional)]
    pub mask_char: Option<String>,
}

/// The tuning of an [`Operator::truncate`] operator.
#[derive(Default, Deserialize, Tsify)]
#[serde(default, rename_all = "camelCase")]
pub struct TruncateConfig {
    /// Leading characters to keep (default `0`).
    #[tsify(optional)]
    pub keep_prefix: usize,
    /// Trailing characters to keep (default `0`).
    #[tsify(optional)]
    pub keep_suffix: usize,
}

/// One bound of an [`Operator::clamp`] operator.
#[derive(Deserialize, Tsify)]
#[serde(rename_all = "camelCase")]
pub struct ClampBound {
    /// The bound: values at or past it are replaced by its label.
    pub value: f64,
    /// The label's format, `{n}` standing for the bound (default `{n}+` for a
    /// ceiling, `<{n}` for a floor).
    #[tsify(optional)]
    pub format: Option<String>,
}

/// The tuning of an [`Operator::clamp`] operator: at least one bound.
#[derive(Deserialize, Tsify)]
#[serde(rename_all = "camelCase")]
pub struct ClampConfig {
    /// Values at or below this bound become its label.
    #[tsify(optional)]
    pub floor: Option<ClampBound>,
    /// Values at or above this bound become its label.
    #[tsify(optional)]
    pub ceiling: Option<ClampBound>,
}

/// How finely an [`Operator::generalize_date`] operator keeps a date.
#[derive(Default, Clone, Copy, Deserialize, Tsify)]
#[serde(rename_all = "camelCase")]
pub enum DateGranularity {
    /// The year: `1987-03-14` becomes `1987`.
    #[default]
    Year,
    /// The year and month: `1987-03-14` becomes `1987-03`.
    YearMonth,
    /// The date and hour: `1987-03-14T09:32:15` becomes `1987-03-14T09`.
    Hour,
}

impl From<DateGranularity> for RustDateGranularity {
    fn from(granularity: DateGranularity) -> Self {
        match granularity {
            DateGranularity::Year => Self::Year,
            DateGranularity::YearMonth => Self::YearMonth,
            DateGranularity::Hour => Self::Hour,
        }
    }
}

/// Which written date convention an [`Operator::generalize_date`] operator
/// reads.
#[derive(Default, Clone, Copy, Deserialize, Tsify)]
#[serde(rename_all = "camelCase")]
pub enum DateStyle {
    /// ISO 8601 only, like `1987-03-14`.
    #[default]
    Iso,
    /// US month-first slashes, like `03/14/1987`, and ISO 8601.
    Us,
}

impl From<DateStyle> for RustDateStyle {
    fn from(style: DateStyle) -> Self {
        match style {
            DateStyle::Iso => Self::Iso,
            DateStyle::Us => Self::Us,
        }
    }
}

/// The tuning of an [`Operator::generalize_date`] operator.
#[derive(Default, Deserialize, Tsify)]
#[serde(default, rename_all = "camelCase")]
pub struct GeneralizeDateConfig {
    /// How finely to keep the date (default `year`).
    #[tsify(optional)]
    pub granularity: DateGranularity,
    /// Which convention to read dates in (default `iso`).
    #[tsify(optional)]
    pub style: DateStyle,
}

/// The tuning of an [`Operator::fake`] operator.
#[derive(Default, Deserialize, Tsify)]
#[serde(default, rename_all = "camelCase")]
pub struct FakeConfig {
    /// Salts the generated values (default `0`). Each entity's fake is drawn
    /// from its own id, so mentions of one entity fake alike within a run.
    #[tsify(optional)]
    pub seed: u32,
    /// The language (a BCP-47 tag) to fake in when an entity has none
    /// (default `en`).
    #[tsify(optional)]
    pub default_language: Option<String>,
}

/// The SHA-2 variant a hash operator uses.
#[derive(Default, Clone, Copy, Deserialize, Tsify)]
#[serde(rename_all = "camelCase")]
pub enum Sha2Algorithm {
    /// SHA-256: 64 hex characters.
    #[default]
    Sha256,
    /// SHA-512: 128 hex characters.
    Sha512,
}

impl From<Sha2Algorithm> for RustSha2Algorithm {
    fn from(algorithm: Sha2Algorithm) -> Self {
        match algorithm {
            Sha2Algorithm::Sha256 => Self::Sha256,
            Sha2Algorithm::Sha512 => Self::Sha512,
        }
    }
}

/// The tuning of an [`Operator::hash`] operator.
#[derive(Default, Deserialize, Tsify)]
#[serde(default, rename_all = "camelCase")]
pub struct HashConfig {
    /// The SHA-2 variant (default `sha256`).
    #[tsify(optional)]
    pub algorithm: Sha2Algorithm,
    /// Bytes mixed into every hash, so a value cannot be looked up in a table
    /// of precomputed hashes (default none).
    #[serde(with = "serde_bytes")]
    #[tsify(type = "Uint8Array", optional)]
    pub salt: Option<Vec<u8>>,
}

/// The tuning of an [`Operator::hmac`] operator.
#[derive(Default, Deserialize, Tsify)]
#[serde(default, rename_all = "camelCase")]
pub struct HmacConfig {
    /// The SHA-2 variant (default `sha256`).
    #[tsify(optional)]
    pub algorithm: Sha2Algorithm,
}

/// The tone shape of an [`Operator::beep`] operator.
#[derive(Default, Clone, Copy, Deserialize, Tsify)]
#[serde(rename_all = "camelCase")]
pub enum Waveform {
    /// A pure sine, the broadcast censor beep.
    #[default]
    Sine,
    /// A square wave, harsher.
    Square,
}

impl From<Waveform> for RustWaveform {
    fn from(waveform: Waveform) -> Self {
        match waveform {
            Waveform::Sine => Self::Sine,
            Waveform::Square => Self::Square,
        }
    }
}

/// The tuning of an [`Operator::beep`] operator.
#[derive(Default, Deserialize, Tsify)]
#[serde(default, rename_all = "camelCase")]
pub struct BeepConfig {
    /// The tone's frequency in hertz (default `1000`).
    #[tsify(optional)]
    pub hz: Option<f32>,
    /// The peak amplitude, in `0..=1` of full scale (default `0.5`).
    #[tsify(optional)]
    pub amplitude: Option<f32>,
    /// The tone's shape (default `sine`).
    #[tsify(optional)]
    pub waveform: Waveform,
}

/// The operators an [`Operator`] handle may carry, one variant per shipped kind.
pub(crate) enum OperatorKind {
    /// Substitute a fixed template string (text, tabular).
    Replace(Replace),
    /// Mask characters, optionally keeping a head or tail (text, tabular).
    Mask(Mask),
    /// Keep a head and a tail, dropping the middle (text, tabular).
    Truncate(Truncate),
    /// Replace an out-of-range number by a bucket label (text, tabular).
    Clamp(Clamp),
    /// Reduce a date to a coarser granularity (text, tabular).
    GeneralizeDate(GeneralizeDate),
    /// Replace by a realistic fake, or by the fallback for a label it cannot
    /// fake (text, tabular).
    Fake {
        fallback: Box<OperatorKind>,
        seed: u64,
        default_language: Option<LanguageTag>,
    },
    /// Replace by a salted SHA-2 hash (text, tabular).
    Hash(Sha2Hash),
    /// Replace by a keyed HMAC (text, tabular).
    Hmac(HmacHash),
    /// Replace by its AES-256-GCM encryption (text, tabular).
    Encrypt(AesEncrypt),
    /// Remove the matched value entirely (text, tabular, audio).
    Remove,
    /// Hide the matched value in place, keeping its extent (text, tabular,
    /// image, audio).
    Cover,
    /// Leave the matched value untouched (text, tabular, image).
    Keep,
    /// Drop the matched cell's whole row (tabular).
    DropRow,
    /// Drop the matched cell's whole column (tabular).
    DropColumn,
    /// Blur the matched region (image).
    Blur(Blur),
    /// Pixelate the matched region (image).
    Pixelate(Pixelate),
    /// Replace the matched span by a tone (audio).
    Beep(Beep),
}

impl OperatorKind {
    /// The operator's JS constructor name, for a mismatch error message.
    fn name(&self) -> &'static str {
        match self {
            Self::Replace(_) => "replace",
            Self::Mask(_) => "mask",
            Self::Truncate(_) => "truncate",
            Self::Clamp(_) => "clamp",
            Self::GeneralizeDate(_) => "generalizeDate",
            Self::Fake { .. } => "fake",
            Self::Hash(_) => "hash",
            Self::Hmac(_) => "hmac",
            Self::Encrypt(_) => "encrypt",
            Self::Remove => "remove",
            Self::Cover => "cover",
            Self::Keep => "keep",
            Self::DropRow => "dropRow",
            Self::DropColumn => "dropColumn",
            Self::Blur(_) => "blur",
            Self::Pixelate(_) => "pixelate",
            Self::Beep(_) => "beep",
        }
    }

    /// The error for an operator used in an anonymizer stage it cannot run in.
    fn mismatch(&self, modality: &str) -> ElideError {
        ElideError::configuration(format!(
            "the {} operator does not apply to the {modality} modality",
            self.name()
        ))
    }
}

/// A redaction operator, ready to fold into an anonymizer rule.
///
/// Built with one of the static constructors and consumed by
/// [`Rule::label`](crate::rule::Rule::label) or
/// [`Rule::fallback`](crate::rule::Rule::fallback).
#[wasm_bindgen]
pub struct Operator(OperatorKind);

#[wasm_bindgen]
impl Operator {
    /// Substitute a fixed `template` for the matched value (text, tabular).
    #[wasm_bindgen(js_name = replace)]
    pub fn replace(template: String) -> Operator {
        Self(OperatorKind::Replace(Replace::new(template)))
    }

    /// Mask the matched value per `config` (text, tabular).
    ///
    /// # Errors
    ///
    /// Rejects if `config` is not a valid mask configuration.
    #[wasm_bindgen(js_name = mask)]
    pub fn mask(config: Ts<MaskConfig>) -> Result<Operator, ElideError> {
        let config = from_js(&config, "mask config")?;
        let mask_char = config
            .mask_char
            .and_then(|mask_char| mask_char.chars().next())
            .unwrap_or('*');
        let mask = Mask::new(mask_char)
            .with_keep_prefix(config.keep_prefix)
            .with_keep_suffix(config.keep_suffix);
        Ok(Self(OperatorKind::Mask(mask)))
    }

    /// Keep the matched value's head and tail per `config` and drop the middle,
    /// like `4111…1111` (text, tabular).
    ///
    /// # Errors
    ///
    /// Rejects if `config` is not a valid truncate configuration.
    #[wasm_bindgen(js_name = truncate)]
    pub fn truncate(config: Ts<TruncateConfig>) -> Result<Operator, ElideError> {
        let config = from_js(&config, "truncate config")?;
        Ok(Self(OperatorKind::Truncate(Truncate::new(
            config.keep_prefix,
            config.keep_suffix,
        ))))
    }

    /// Replace a number at or past a bound by the bound's label, like an age
    /// of `94` by `90+` (text, tabular). A number within the bounds is left as
    /// it is; a value that is not a number is removed.
    ///
    /// # Errors
    ///
    /// Rejects if `config` is not a valid clamp configuration, sets no bound,
    /// sets a bound that is not finite, or a floor not below its ceiling.
    #[wasm_bindgen(js_name = clamp)]
    pub fn clamp(config: Ts<ClampConfig>) -> Result<Operator, ElideError> {
        let config = from_js(&config, "clamp config")?;
        if config.floor.is_none() && config.ceiling.is_none() {
            return Err(ElideError::configuration(
                "a clamp needs a floor or a ceiling",
            ));
        }
        let bounds = [&config.floor, &config.ceiling];
        if bounds
            .iter()
            .flat_map(|bound| bound.iter())
            .any(|bound| !bound.value.is_finite())
        {
            return Err(ElideError::configuration(
                "a clamp bound must be a finite number",
            ));
        }
        if let (Some(floor), Some(ceiling)) = (&config.floor, &config.ceiling)
            && floor.value >= ceiling.value
        {
            return Err(ElideError::configuration(
                "a clamp's floor must be below its ceiling",
            ));
        }
        let mut clamp = Clamp::new();
        if let Some(floor) = config.floor {
            clamp = match floor.format {
                Some(format) => clamp.with_floor_fmt(floor.value, format),
                None => clamp.with_floor_auto(floor.value),
            };
        }
        if let Some(ceiling) = config.ceiling {
            clamp = match ceiling.format {
                Some(format) => clamp.with_ceiling_fmt(ceiling.value, format),
                None => clamp.with_ceiling_auto(ceiling.value),
            };
        }
        Ok(Self(OperatorKind::Clamp(clamp)))
    }

    /// Reduce a date or timestamp to a coarser granularity, like `1987-03-14`
    /// to `1987` (text, tabular). A value that is not a date in the configured
    /// style is removed. Without `config`, it keeps the year of an ISO
    /// 8601 date.
    ///
    /// # Errors
    ///
    /// Rejects if `config` is not a valid date configuration.
    #[wasm_bindgen(js_name = generalizeDate)]
    pub fn generalize_date(
        config: Option<Ts<GeneralizeDateConfig>>,
    ) -> Result<Operator, ElideError> {
        let config = from_js_or_default(config.as_ref(), "date config")?;
        let generalize =
            GeneralizeDate::new(config.granularity.into()).with_style(config.style.into());
        Ok(Self(OperatorKind::GeneralizeDate(generalize)))
    }

    /// Replace the matched value by a realistic fake of its label, like a
    /// made-up name for a name, keeping a structured value's shape (text,
    /// tabular). A label it cannot fake gets `fallback`, a text operator.
    ///
    /// # Errors
    ///
    /// Rejects if `config` is not a valid fake configuration, or its default
    /// language is not a valid BCP-47 tag.
    #[wasm_bindgen(js_name = fake)]
    pub fn fake(
        fallback: Operator,
        config: Option<Ts<FakeConfig>>,
    ) -> Result<Operator, ElideError> {
        let config = from_js_or_default(config.as_ref(), "fake config")?;
        let default_language = config
            .default_language
            .map(|tag| language_tag(&tag))
            .transpose()?;
        Ok(Self(OperatorKind::Fake {
            fallback: Box::new(fallback.0),
            seed: config.seed.into(),
            default_language,
        }))
    }

    /// Replace the matched value by its SHA-2 hash in hex (text, tabular).
    ///
    /// A short value, like a phone number, can be recovered from its hash by
    /// hashing every candidate, salt or not; prefer [`hmac`](Self::hmac) with
    /// a secret key.
    ///
    /// # Errors
    ///
    /// Rejects if `config` is not a valid hash configuration.
    #[wasm_bindgen(js_name = hash)]
    pub fn hash(config: Option<Ts<HashConfig>>) -> Result<Operator, ElideError> {
        let config = from_js_or_default(config.as_ref(), "hash config")?;
        let hash = Sha2Hash::new(config.algorithm.into());
        let hash = match config.salt {
            Some(salt) => hash.with_salt(salt),
            None => hash,
        };
        Ok(Self(OperatorKind::Hash(hash)))
    }

    /// Replace the matched value by its HMAC under the secret `key`, in hex
    /// (text, tabular). The same value under the same key gives the same
    /// output, so records stay linkable without exposing the value.
    ///
    /// # Errors
    ///
    /// Rejects an empty `key`, or a `config` that is not a valid HMAC
    /// configuration.
    #[wasm_bindgen(js_name = hmac)]
    pub fn hmac(key: Vec<u8>, config: Option<Ts<HmacConfig>>) -> Result<Operator, ElideError> {
        if key.is_empty() {
            return Err(ElideError::configuration("an HMAC key must not be empty"));
        }
        let config = from_js_or_default(config.as_ref(), "HMAC config")?;
        let hmac = match config.algorithm {
            Sha2Algorithm::Sha256 => HmacHash::sha256(key),
            Sha2Algorithm::Sha512 => HmacHash::sha512(key),
        };
        Ok(Self(OperatorKind::Hmac(hmac)))
    }

    /// Replace the matched value by its AES-256-GCM encryption under `key`,
    /// base64-encoded, so the key's holder can recover it (text, tabular).
    ///
    /// # Errors
    ///
    /// Rejects a `key` that is not 32 bytes.
    #[wasm_bindgen(js_name = encrypt)]
    pub fn encrypt(key: Vec<u8>) -> Result<Operator, ElideError> {
        if key.len() != AES_KEY_LEN {
            return Err(ElideError::configuration(format!(
                "an AES-256 key is {AES_KEY_LEN} bytes, not {}",
                key.len()
            )));
        }
        Ok(Self(OperatorKind::Encrypt(AesEncrypt::with_key(key))))
    }

    /// Remove the matched value entirely (text, tabular, audio).
    #[wasm_bindgen(js_name = remove)]
    pub fn remove() -> Operator {
        Self(OperatorKind::Remove)
    }

    /// Hide the matched value in place, keeping its extent: a run of `█` in
    /// text, a black box in an image, silence in audio.
    #[wasm_bindgen(js_name = cover)]
    pub fn cover() -> Operator {
        Self(OperatorKind::Cover)
    }

    /// Leave the matched value untouched (text, tabular, image).
    #[wasm_bindgen(js_name = keep)]
    pub fn keep() -> Operator {
        Self(OperatorKind::Keep)
    }

    /// Drop the whole row of the matched cell (tabular).
    #[wasm_bindgen(js_name = dropRow)]
    pub fn drop_row() -> Operator {
        Self(OperatorKind::DropRow)
    }

    /// Drop the whole column of the matched cell (tabular).
    #[wasm_bindgen(js_name = dropColumn)]
    pub fn drop_column() -> Operator {
        Self(OperatorKind::DropColumn)
    }

    /// Blur the matched region with a Gaussian of `sigma` pixels (image).
    ///
    /// # Errors
    ///
    /// Rejects a `sigma` below `1`, which leaves the region readable.
    #[wasm_bindgen(js_name = blur)]
    pub fn blur(sigma: f32) -> Result<Operator, ElideError> {
        Ok(Self(OperatorKind::Blur(Blur::new(sigma)?)))
    }

    /// Pixelate the matched region in blocks of `blockSize` pixels (image).
    ///
    /// # Errors
    ///
    /// Rejects a block size below `2`, which leaves the region as it is.
    #[wasm_bindgen(js_name = pixelate)]
    pub fn pixelate(block_size: u32) -> Result<Operator, ElideError> {
        Ok(Self(OperatorKind::Pixelate(Pixelate::new(block_size)?)))
    }

    /// Replace the matched span by a tone, keeping the timeline (audio).
    ///
    /// # Errors
    ///
    /// Rejects if `config` is not a valid beep configuration, or its frequency
    /// is not positive.
    #[wasm_bindgen(js_name = beep)]
    pub fn beep(config: Option<Ts<BeepConfig>>) -> Result<Operator, ElideError> {
        let config = from_js_or_default(config.as_ref(), "beep config")?;
        let beep = match config.hz {
            Some(hz) => Beep::new(hz)?,
            None => Beep::default(),
        };
        let beep = match config.amplitude {
            Some(amplitude) => beep.with_amplitude(amplitude),
            None => beep,
        };
        Ok(Self(OperatorKind::Beep(
            beep.with_waveform(config.waveform.into()),
        )))
    }
}

impl Operator {
    /// The wrapped operator kind, consumed when folded into a rule.
    pub(crate) fn into_kind(self) -> OperatorKind {
        self.0
    }
}

/// A boxed operator for modality `M`.
type Boxed<M> = Box<dyn RustOperator<M>>;

impl OperatorKind {
    /// This operator for the text-valued `modality` `M`.
    ///
    /// # Errors
    ///
    /// It, or a fake's fallback, is not a text operator.
    fn textual<M>(self, modality: &str) -> Result<Boxed<M>, ElideError>
    where
        M: TextRedactable,
        Remove: RustOperator<M>,
        Cover: RustOperator<M>,
    {
        let operator: Boxed<M> = match self {
            Self::Replace(replace) => Box::new(replace),
            Self::Mask(mask) => Box::new(mask),
            Self::Truncate(truncate) => Box::new(truncate),
            Self::Clamp(clamp) => Box::new(clamp),
            Self::GeneralizeDate(generalize) => Box::new(generalize),
            Self::Hash(hash) => Box::new(hash),
            Self::Hmac(hmac) => Box::new(hmac),
            Self::Encrypt(encrypt) => Box::new(encrypt),
            Self::Remove => Box::new(Remove),
            Self::Cover => Box::new(Cover::default()),
            Self::Keep => Box::new(Keep),
            Self::Fake {
                fallback,
                seed,
                default_language,
            } => {
                // A nested fake reports its own fallback; any other non-text
                // fallback is named as the fake's.
                let name = fallback.name();
                let nested = matches!(*fallback, Self::Fake { .. });
                let fallback = fallback.textual::<M>(modality).map_err(|error| {
                    if nested {
                        error
                    } else {
                        ElideError::configuration(format!(
                            "a fake's fallback must be a text operator, not {name}"
                        ))
                    }
                })?;
                let fake = Fake::new(fallback).with_seed(seed);
                match default_language {
                    Some(language) => Box::new(fake.with_default_language(language)),
                    None => Box::new(fake),
                }
            }
            other => return Err(other.mismatch(modality)),
        };
        Ok(operator)
    }

    /// Build a `Text` rule from this operator: a label rule when `label` is
    /// `Some`, else a fallback rule.
    pub(crate) fn text_rule(self, label: Option<LabelRef>) -> Result<RustRule<Text>, ElideError> {
        Ok(rule(label, self.textual::<Text>("text")?))
    }

    /// Build a `Tabular` rule from this operator.
    pub(crate) fn tabular_rule(
        self,
        label: Option<LabelRef>,
    ) -> Result<RustRule<Tabular>, ElideError> {
        match self {
            Self::DropRow => Ok(rule(label, DropRow)),
            Self::DropColumn => Ok(rule(label, DropColumn)),
            other => Ok(rule(label, other.textual::<Tabular>("tabular")?)),
        }
    }

    /// Build an `Image` rule from this operator.
    pub(crate) fn image_rule(self, label: Option<LabelRef>) -> Result<RustRule<Image>, ElideError> {
        match self {
            Self::Cover => Ok(rule(label, Cover::default())),
            Self::Keep => Ok(rule(label, Keep)),
            Self::Blur(blur) => Ok(rule(label, blur)),
            Self::Pixelate(pixelate) => Ok(rule(label, pixelate)),
            other => Err(other.mismatch("image")),
        }
    }

    /// Build an `Audio` rule from this operator.
    pub(crate) fn audio_rule(self, label: Option<LabelRef>) -> Result<RustRule<Audio>, ElideError> {
        match self {
            Self::Cover => Ok(rule(label, Cover::default())),
            Self::Remove => Ok(rule(label, Remove)),
            Self::Beep(beep) => Ok(rule(label, beep)),
            other => Err(other.mismatch("audio")),
        }
    }
}

/// A label rule when `label` is `Some`, else a fallback rule, for `operator`.
fn rule<M, O>(label: Option<LabelRef>, operator: O) -> RustRule<M>
where
    M: Modality,
    O: RustOperator<M> + 'static,
{
    match label {
        Some(label) => RustRule::label(label, operator),
        None => RustRule::fallback(operator),
    }
}
