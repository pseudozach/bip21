//! Rust-idiomatic, compliant, flexible and performant BIP21 crate.
//!
//! **Important:** while lot of work went into polishing the crate it's still considered
//! early-development!
//!
//! * Rust-idiomatic: uses strong types, standard traits and other things
//! * Compliant: implements all requirements of BIP21, including protections to not forget about
//!              `req-`. (But see features.)
//! * Flexible: enables parsing/serializing additional arguments not defined by BIP21
//! * Performant: uses zero-copy deserialization and lazy evaluation wherever possible.
//!
//! Serialization and deserialization is inspired by `serde` with these important differences:
//!
//! * Deserialization signals if the field is known so that `req-` fields can be rejected.
//! * Much simpler API - we don't need all the features.
//! * Use of [`Param<'a>`] to enable lazy evaluation.
//!
//! The crate is `no_std` but does require `alloc`.
//!
//! ## Composable Extras
//!
//! Modern BIP21 usage often requires supporting multiple parameter extensions from different
//! sources (e.g., Lightning Network, Payjoin, Silent Payments). This crate supports composing
//! multiple `Extras` implementations using tuples:
//!
//! ```ignore
//! // Example: Compose Lightning and Payjoin extras
//! type MyExtras = (LightningExtras, PayjoinExtras);
//! let uri: Uri<'_, _, MyExtras> = uri_string.parse()?;
//! ```
//!
//! This allows each parameter set to be implemented and maintained in its own crate, then
//! composed as needed downstream without duplicating deserialization/serialization logic.
//!
//! ## Features
//!
//! * `std` enables integration with `std` - mainly `std::error::Error`.
//! * `non-compliant-bytes` - enables use of non-compliant API that can parse non-UTF-8 URI values.
//!
//! ## Stabilization roadmap
//!
//! The crate can not (and will not) be stabilized until either [`bitcoin`] is stabilized or
//! [`bitcoin::Address`] and [`bitcoin::Amount`] are moved to (a) separate stable crate(s).
//!
//! ## MSRV
//!
//! 1.56.1

#![cfg_attr(docsrs, feature(doc_cfg))]
#![no_std]
#![deny(unused_must_use)]
#![deny(missing_docs)]

#[cfg(feature = "std")]
extern crate std;

extern crate alloc;

pub mod de;
pub mod ser;

use alloc::borrow::ToOwned;
use alloc::borrow::Cow;
#[cfg(feature = "non-compliant-bytes")]
use alloc::vec::Vec;
use alloc::string::String;
use percent_encoding_rfc3986::{PercentDecode, PercentDecodeError};
#[cfg(feature = "non-compliant-bytes")]
use either::Either;
use core::convert::{TryFrom, TryInto};
use core::fmt;
use bitcoin::address::NetworkValidation;

pub use de::{DeserializeParams, DeserializationState, DeserializationError};
pub use ser::SerializeParams;

/// Parsed BIP21 URI.
///
/// This struct represents all fields of BIP21 URI with the ability to add more extra fields using
/// the `extras` field. By default there are no extra fields so an empty implementation is used.
///
/// ## Parsing
///
/// `Uri` implements `FromStr` so you can simply use `s.parse::<Uri<'static>>()`. However that is
/// not zero-copy. If you wish to use zero-copy parsing call `try_into()` instead.
///
/// ## Displaying
///
/// `Display` is implemented for `Uri` so you can format it naturally. However it currently does
/// **not** support alignment.
///
/// The code does _not_ assume strict BIP-21, so it displays the schema lower case
/// instead as upper case.
/// This makes it compatible with some (buggy) wallets but does not create the most optimal QR codes.
///
/// [See compatibility table.](https://github.com/btcpayserver/btcpayserver/issues/2110)
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct Uri<'a, NetVal = bitcoin::address::NetworkChecked, Extras = NoExtras>
where
    NetVal: NetworkValidation,
{
    /// The address provided in the URI.
    ///
    /// This field is mandatory because the address is mandatory in BIP21.
    pub address: bitcoin::Address<NetVal>,

    /// Number of satoshis requested as payment.
    pub amount: Option<bitcoin::Amount>,

    /// The label of the address - e.g. name of the receiver.
    pub label: Option<Param<'a>>,

    /// Message that describes the transaction to the user.
    pub message: Option<Param<'a>>,

    /// Extra fields that can occur in a BIP21 URI.
    pub extras: Extras,
}

impl<'a, NetVal: NetworkValidation, T: Default> Uri<'a, NetVal, T> {
    /// Creates an URI with defaults.
    ///
    /// This sets all fields except `address` to default values.
    /// They can be overwritten in subsequent assignments before displaying the URI.
    pub fn new(address: bitcoin::Address<NetVal>) -> Self {
        Uri {
            address,
            amount: None,
            label: None,
            message: None,
            extras: Default::default(),
        }
    }
}

impl<'a, NetVal: NetworkValidation, T> Uri<'a, NetVal, T> {
    /// Creates an URI with defaults.
    ///
    /// This sets all fields except `address` and `extras` to default values.
    /// They can be overwritten in subsequent assignments before displaying the URI.
    pub fn with_extras(address: bitcoin::Address<NetVal>, extras: T) -> Self {
        Uri {
            address,
            amount: None,
            label: None,
            message: None,
            extras,
        }
    }
}

/// Abstracted stringly parameter in the URI.
///
/// This type abstracts the parameter that may be encoded allowing lazy decoding, possibly even
/// without allocation.
/// When constructing [`Uri`] to be displayed you may use `From<S>` where `S` is one of various
/// stringly types. The conversion is always cheap.
#[derive(Debug, Clone)]
pub struct Param<'a>(ParamInner<'a>);

impl<'a> Param<'a> {
    /// Convenience constructor.
    fn decode(s: &'a str) -> Result<Self, PercentDecodeError> {
        Ok(Param(ParamInner::EncodedBorrowed(percent_encoding_rfc3986::percent_decode_str(s)?)))
    }

    /// Creates a byte iterator yielding decoded bytes.
    #[cfg(feature = "non-compliant-bytes")]
    #[cfg_attr(docsrs, doc(cfg(feature = "non-compliant-bytes")))]
    pub fn bytes(&self) -> ParamBytes<'_> {
        ParamBytes(match &self.0 {
            ParamInner::EncodedBorrowed(decoder) => Either::Left(decoder.clone()),
            ParamInner::UnencodedBytes(bytes) => Either::Right(bytes.iter().cloned()),
            ParamInner::UnencodedString(string) => Either::Right(string.as_bytes().iter().cloned()),
        })
    }

    /// Converts the parameter into iterator yielding decoded bytes.
    #[cfg(feature = "non-compliant-bytes")]
    #[cfg_attr(docsrs, doc(cfg(feature = "non-compliant-bytes")))]
    pub fn into_bytes(self) -> ParamBytesOwned<'a> {
        ParamBytesOwned(match self.0 {
            ParamInner::EncodedBorrowed(decoder) => Either::Left(decoder),
            ParamInner::UnencodedBytes(Cow::Borrowed(bytes)) => Either::Right(Either::Left(bytes.iter().cloned())),
            ParamInner::UnencodedBytes(Cow::Owned(bytes)) => Either::Right(Either::Right(bytes.into_iter())),
            ParamInner::UnencodedString(Cow::Borrowed(string)) => Either::Right(Either::Left(string.as_bytes().iter().cloned())),
            ParamInner::UnencodedString(Cow::Owned(string)) => Either::Right(Either::Right(Vec::from(string).into_iter())),
        })
    }

    /// Decodes the param if encoded making the lifetime static.
    fn decode_into_owned<'b>(self) -> Param<'b> {
        let owned = match self.0 {
            ParamInner::EncodedBorrowed(decoder) => ParamInner::UnencodedBytes(decoder.collect()),
            ParamInner::UnencodedString(Cow::Borrowed(value)) => ParamInner::UnencodedString(Cow::Owned(value.to_owned())),
            ParamInner::UnencodedString(Cow::Owned(value)) => ParamInner::UnencodedString(Cow::Owned(value)),
            ParamInner::UnencodedBytes(Cow::Borrowed(value)) => ParamInner::UnencodedBytes(Cow::Owned(value.to_owned())),
            ParamInner::UnencodedBytes(Cow::Owned(value)) => ParamInner::UnencodedBytes(Cow::Owned(value)),
        };
        Param(owned)
    }
}

/// Cheap conversion
impl<'a> From<&'a str> for Param<'a> {
    fn from(value: &'a str) -> Self {
        Param(ParamInner::UnencodedString(Cow::Borrowed(value)))
    }
}

/// Cheap conversion
impl<'a> From<String> for Param<'a> {
    fn from(value: String) -> Self {
        Param(ParamInner::UnencodedString(Cow::Owned(value)))
    }
}

/// Cheap conversion
#[cfg(feature = "non-compliant-bytes")]
#[cfg_attr(docsrs, doc(cfg(feature = "non-compliant-bytes")))]
impl<'a> From<&'a [u8]> for Param<'a> {
    fn from(value: &'a [u8]) -> Self {
        Param(ParamInner::UnencodedBytes(Cow::Borrowed(value)))
    }
}

/// Cheap conversion
#[cfg(feature = "non-compliant-bytes")]
#[cfg_attr(docsrs, doc(cfg(feature = "non-compliant-bytes")))]
impl<'a> From<Vec<u8>> for Param<'a> {
    fn from(value: Vec<u8>) -> Self {
        Param(ParamInner::UnencodedBytes(Cow::Owned(value)))
    }
}

/// Cheap conversion
#[cfg(feature = "non-compliant-bytes")]
#[cfg_attr(docsrs, doc(cfg(feature = "non-compliant-bytes")))]
impl<'a> From<Param<'a>> for Vec<u8> {
    fn from(value: Param<'a>) -> Self {
        match value.0 {
            ParamInner::EncodedBorrowed(decoder) => decoder.collect(),
            ParamInner::UnencodedString(Cow::Borrowed(value)) => value.as_bytes().to_owned(),
            ParamInner::UnencodedString(Cow::Owned(value)) => value.into(),
            ParamInner::UnencodedBytes(value) => value.into(),
        }
    }
}

/// Cheap conversion
#[cfg(feature = "non-compliant-bytes")]
#[cfg_attr(docsrs, doc(cfg(feature = "non-compliant-bytes")))]
impl<'a> From<Param<'a>> for Cow<'a, [u8]> {
    fn from(value: Param<'a>) -> Self {
        match value.0 {
            ParamInner::EncodedBorrowed(decoder) => decoder.into(),
            ParamInner::UnencodedString(Cow::Borrowed(value)) => Cow::Borrowed(value.as_bytes()),
            ParamInner::UnencodedString(Cow::Owned(value)) => Cow::Owned(value.into()),
            ParamInner::UnencodedBytes(value) => value,
        }
    }
}

impl<'a> TryFrom<Param<'a>> for String {
    type Error = core::str::Utf8Error;

    fn try_from(value: Param<'a>) -> Result<Self, Self::Error> {
        match value.0 {
            ParamInner::EncodedBorrowed(decoder) => <Cow<'_, str>>::try_from(decoder).map(Into::into),
            ParamInner::UnencodedString(value) => Ok(value.into()),
            ParamInner::UnencodedBytes(Cow::Borrowed(value)) => Ok(core::str::from_utf8(value)?.to_owned()),
            ParamInner::UnencodedBytes(Cow::Owned(value)) => String::from_utf8(value).map_err(|error| error.utf8_error()),
        }
    }
}

impl<'a> TryFrom<Param<'a>> for Cow<'a, str> {
    type Error = core::str::Utf8Error;

    fn try_from(value: Param<'a>) -> Result<Self, Self::Error> {
        match value.0 {
            ParamInner::EncodedBorrowed(decoder) => decoder.try_into(),
            ParamInner::UnencodedString(value) => Ok(value),
            ParamInner::UnencodedBytes(Cow::Borrowed(value)) => Ok(Cow::Borrowed(core::str::from_utf8(value)?)),
            ParamInner::UnencodedBytes(Cow::Owned(value)) => Ok(Cow::Owned(String::from_utf8(value).map_err(|error| error.utf8_error())?)),
        }
    }
}

#[derive(Debug, Clone)]
enum ParamInner<'a> {
    EncodedBorrowed(PercentDecode<'a>),
    UnencodedBytes(Cow<'a, [u8]>),
    UnencodedString(Cow<'a, str>),
}

/// Iterator over decoded bytes inside paramter.
///
/// The lifetime of this may be shorter than that of [`Param<'a>`].
#[cfg(feature = "non-compliant-bytes")]
#[cfg_attr(docsrs, doc(cfg(feature = "non-compliant-bytes")))]
pub struct ParamBytes<'a>(ParamIterInner<'a, core::iter::Cloned<core::slice::Iter<'a, u8>>>);

/// Iterator over decoded bytes inside paramter.
///
/// The lifetime of this is same as that of [`Param<'a>`].
#[cfg(feature = "non-compliant-bytes")]
#[cfg_attr(docsrs, doc(cfg(feature = "non-compliant-bytes")))]
pub struct ParamBytesOwned<'a>(ParamIterInner<'a, Either<core::iter::Cloned<core::slice::Iter<'a, u8>>, alloc::vec::IntoIter<u8>>>);

#[cfg(feature = "non-compliant-bytes")]
type ParamIterInner<'a, T> = either::Either<PercentDecode<'a>, T>;

/// Empty extras.
///
/// This type can be used if extras are not required.
/// It is also the default type parameter of [`Uri`].
#[derive(Debug, Default, Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct NoExtras;

/// This is a state used to deserialize `NoExtras` - it doesn't expect any parameters.
#[derive(Debug, Default, Copy, Clone)]
pub struct EmptyState;

impl DeserializeParams<'_> for NoExtras {
    type DeserializationState = EmptyState;
}

impl DeserializationError for NoExtras {
    type Error = core::convert::Infallible;
}

impl<'de> DeserializationState<'de> for EmptyState {
    type Value = NoExtras;

    fn is_param_known(&self, _key: &str) -> bool {
        false
    }

    fn deserialize_temp(&mut self, _key: &str, _value: Param<'_>) -> Result<de::ParamKind, <Self::Value as DeserializationError>::Error> {
        Ok(de::ParamKind::Unknown)
    }

    fn finalize(self) -> Result<Self::Value, <Self::Value as DeserializationError>::Error> {
        Ok(Default::default())
    }
}

impl<'a> SerializeParams for &'a NoExtras {
    type Key = core::convert::Infallible;
    type Value = core::convert::Infallible;
    type Iterator = core::iter::Empty<(Self::Key, Self::Value)>;

    fn serialize_params(self) -> Self::Iterator {
        core::iter::empty()
    }
}

// Composable extras implementation for tuples.
// This allows combining multiple Extras implementations.

/// Error type for combining two different extras types.
///
/// This is used when composing extras with tuples. When either the left or right
/// extras type returns an error during deserialization, it's wrapped in this enum.
///
/// # Examples
///
/// ```ignore
/// type ComposedExtras = (LightningExtras, PayjoinExtras);
/// // If deserialization fails, you'll get an EitherError wrapping the specific error
/// ```
#[derive(Debug, Clone)]
pub enum EitherError<L, R> {
    /// Error from the left (first) extras type.
    Left(L),
    /// Error from the right (second) extras type.
    Right(R),
}

impl<L: fmt::Display, R: fmt::Display> fmt::Display for EitherError<L, R> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            EitherError::Left(err) => write!(f, "{}", err),
            EitherError::Right(err) => write!(f, "{}", err),
        }
    }
}

#[cfg(feature = "std")]
impl<L: std::error::Error + 'static, R: std::error::Error + 'static> std::error::Error for EitherError<L, R> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            EitherError::Left(err) => Some(err),
            EitherError::Right(err) => Some(err),
        }
    }
}

/// Deserialization state for a tuple of two extras types.
///
/// This is used internally to handle deserialization of composed extras.
/// When you compose extras using a tuple like `(ExtrasA, ExtrasB)`, this state
/// manages the deserialization process for both types.
///
/// The state routes each parameter to the appropriate extras type based on
/// which one recognizes it via `is_param_known`.
#[derive(Debug, Default)]
pub struct TupleState<S1, S2> {
    state1: S1,
    state2: S2,
}

impl<L, R> DeserializationError for (L, R)
where
    L: DeserializationError,
    R: DeserializationError,
{
    type Error = EitherError<L::Error, R::Error>;
}

impl<'de, L, R> DeserializeParams<'de> for (L, R)
where
    L: DeserializeParams<'de>,
    R: DeserializeParams<'de>,
{
    type DeserializationState = TupleState<L::DeserializationState, R::DeserializationState>;
}

impl<'de, L, R> DeserializationState<'de> for TupleState<L, R>
where
    L: DeserializationState<'de>,
    R: DeserializationState<'de>,
{
    type Value = (L::Value, R::Value);

    fn is_param_known(&self, key: &str) -> bool {
        self.state1.is_param_known(key) || self.state2.is_param_known(key)
    }

    fn deserialize_temp(&mut self, key: &str, value: Param<'_>) -> Result<de::ParamKind, <Self::Value as DeserializationError>::Error> {
        if self.state1.is_param_known(key) {
            self.state1.deserialize_temp(key, value).map_err(EitherError::Left)
        } else if self.state2.is_param_known(key) {
            self.state2.deserialize_temp(key, value).map_err(EitherError::Right)
        } else {
            Ok(de::ParamKind::Unknown)
        }
    }

    fn deserialize_borrowed(&mut self, key: &'de str, value: Param<'de>) -> Result<de::ParamKind, <Self::Value as DeserializationError>::Error> {
        if self.state1.is_param_known(key) {
            self.state1.deserialize_borrowed(key, value).map_err(EitherError::Left)
        } else if self.state2.is_param_known(key) {
            self.state2.deserialize_borrowed(key, value).map_err(EitherError::Right)
        } else {
            Ok(de::ParamKind::Unknown)
        }
    }

    fn finalize(self) -> Result<Self::Value, <Self::Value as DeserializationError>::Error> {
        let left = self.state1.finalize().map_err(EitherError::Left)?;
        let right = self.state2.finalize().map_err(EitherError::Right)?;
        Ok((left, right))
    }
}

/// Iterator that chains two serialization iterators.
///
/// This is used to combine parameters from two different extras types
/// when serializing a composed URI. It first yields all parameters from
/// the first iterator, then all parameters from the second.
///
/// # Type Parameters
///
/// * `I1` - The first iterator type
/// * `I2` - The second iterator type
pub struct ChainedIterator<I1, I2> {
    iter1: I1,
    iter2: Option<I2>,
}

impl<K, V, I1, I2> Iterator for ChainedIterator<I1, I2>
where
    I1: Iterator<Item = (K, V)>,
    I2: Iterator<Item = (K, V)>,
{
    type Item = (K, V);

    fn next(&mut self) -> Option<Self::Item> {
        match self.iter1.next() {
            Some(item) => Some(item),
            None => self.iter2.as_mut().and_then(|iter| iter.next()),
        }
    }
}

impl<'a, L, R> SerializeParams for &'a (L, R)
where
    &'a L: SerializeParams,
    &'a R: SerializeParams<Key = <&'a L as SerializeParams>::Key, Value = <&'a L as SerializeParams>::Value>,
{
    type Key = <&'a L as SerializeParams>::Key;
    type Value = <&'a L as SerializeParams>::Value;
    type Iterator = ChainedIterator<<&'a L as SerializeParams>::Iterator, <&'a R as SerializeParams>::Iterator>;

    fn serialize_params(self) -> Self::Iterator {
        ChainedIterator {
            iter1: (&self.0).serialize_params(),
            iter2: Some((&self.1).serialize_params()),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::Uri;
    use alloc::string::{String, ToString};
    use alloc::borrow::Cow;
    use core::convert::{TryFrom, TryInto};

    fn check_send_sync<T: Send + Sync>() {}

    #[test]
    fn send_sync() {
        check_send_sync::<crate::de::UriError>();
    }

    // Note: the official test vectors contained an invalid address so it was replaced with the address of Andreas Antonopoulos.

    #[test]
    fn just_address() {
        let input = "bitcoin:1andreas3batLhQa2FawWjeyjCqyBzypd";
        let uri = input.parse::<Uri<'_, _>>().unwrap().require_network(bitcoin::Network::Bitcoin).unwrap();
        assert_eq!(uri.address.to_string(), "1andreas3batLhQa2FawWjeyjCqyBzypd");
        assert!(uri.amount.is_none());
        assert!(uri.label.is_none());
        assert!(uri.message.is_none());

        assert_eq!(uri.to_string(), input);
    }

    #[test]
    fn address_with_name() {
        let input = "bitcoin:1andreas3batLhQa2FawWjeyjCqyBzypd?label=Luke-Jr";
        let uri = input.parse::<Uri<'_, _>>().unwrap().require_network(bitcoin::Network::Bitcoin).unwrap();
        let label: Cow<'_, str> = uri.label.clone().unwrap().try_into().unwrap();
        assert_eq!(uri.address.to_string(), "1andreas3batLhQa2FawWjeyjCqyBzypd");
        assert_eq!(label, "Luke-Jr");
        assert!(uri.amount.is_none());
        assert!(uri.message.is_none());

        assert_eq!(uri.to_string(), input);
    }

    #[allow(clippy::inconsistent_digit_grouping)] // Use sats/bitcoin when grouping.
    #[test]
    fn request_20_point_30_btc_to_luke_dash_jr() {
        let input = "bitcoin:1andreas3batLhQa2FawWjeyjCqyBzypd?amount=20.3&label=Luke-Jr";
        let uri = input.parse::<Uri<'_, _>>().unwrap().require_network(bitcoin::Network::Bitcoin).unwrap();
        let label: Cow<'_, str> = uri.label.clone().unwrap().try_into().unwrap();
        assert_eq!(uri.address.to_string(), "1andreas3batLhQa2FawWjeyjCqyBzypd");
        assert_eq!(label, "Luke-Jr");
        assert_eq!(uri.amount, Some(bitcoin::Amount::from_sat(20_30_000_000)));
        assert!(uri.message.is_none());

        assert_eq!(uri.to_string(), input);
    }

    #[allow(clippy::inconsistent_digit_grouping)] // Use sats/bitcoin when grouping.
    #[test]
    fn request_50_btc_with_message() {
        let input = "bitcoin:1andreas3batLhQa2FawWjeyjCqyBzypd?amount=50&label=Luke-Jr&message=Donation%20for%20project%20xyz";
        let uri = input.parse::<Uri<'_, _>>().unwrap().require_network(bitcoin::Network::Bitcoin).unwrap();
        let label: Cow<'_, str> = uri.label.clone().unwrap().try_into().unwrap();
        let message: Cow<'_, str> = uri.message.clone().unwrap().try_into().unwrap();
        assert_eq!(uri.address.to_string(), "1andreas3batLhQa2FawWjeyjCqyBzypd");
        assert_eq!(uri.amount, Some(bitcoin::Amount::from_sat(50_00_000_000)));
        assert_eq!(label, "Luke-Jr");
        assert_eq!(message, "Donation for project xyz");

        assert_eq!(uri.to_string(), input);
    }

    #[test]
    fn required_not_understood() {
        let input = "bitcoin:1andreas3batLhQa2FawWjeyjCqyBzypd?req-somethingyoudontunderstand=50&req-somethingelseyoudontget=999";
        let uri = input.parse::<Uri<'_, _>>();
        assert!(uri.is_err());
    }

    #[test]
    fn required_understood() {
        let input = "bitcoin:1andreas3batLhQa2FawWjeyjCqyBzypd?somethingyoudontunderstand=50&somethingelseyoudontget=999";
        let uri = input.parse::<Uri<'_, _>>().unwrap().require_network(bitcoin::Network::Bitcoin).unwrap();
        assert_eq!(uri.address.to_string(), "1andreas3batLhQa2FawWjeyjCqyBzypd");
        assert!(uri.amount.is_none());
        assert!(uri.label.is_none());
        assert!(uri.message.is_none());
    }

    // Tests for composable extras

    // Example extras types for testing
    #[derive(Debug, Default, Clone, PartialEq)]
    struct LightningExtras {
        lightning: Option<String>,
    }

    #[derive(Debug, Default)]
    struct LightningState {
        lightning: Option<String>,
    }

    impl crate::de::DeserializationError for LightningExtras {
        type Error = core::convert::Infallible;
    }

    impl crate::de::DeserializeParams<'_> for LightningExtras {
        type DeserializationState = LightningState;
    }

    impl<'de> crate::de::DeserializationState<'de> for LightningState {
        type Value = LightningExtras;

        fn is_param_known(&self, key: &str) -> bool {
            key == "lightning"
        }

        fn deserialize_temp(&mut self, key: &str, value: crate::Param<'_>) -> Result<crate::de::ParamKind, <Self::Value as crate::de::DeserializationError>::Error> {
            if key == "lightning" {
                self.lightning = Some(String::try_from(value).unwrap());
                Ok(crate::de::ParamKind::Known)
            } else {
                Ok(crate::de::ParamKind::Unknown)
            }
        }

        fn finalize(self) -> Result<Self::Value, <Self::Value as crate::de::DeserializationError>::Error> {
            Ok(LightningExtras {
                lightning: self.lightning,
            })
        }
    }

    impl<'a> crate::ser::SerializeParams for &'a LightningExtras {
        type Key = &'static str;
        type Value = alloc::string::String;
        type Iterator = alloc::vec::IntoIter<(Self::Key, Self::Value)>;

        fn serialize_params(self) -> Self::Iterator {
            let mut params = alloc::vec::Vec::new();
            if let Some(ref lightning) = self.lightning {
                params.push(("lightning", lightning.clone()));
            }
            params.into_iter()
        }
    }

    #[derive(Debug, Default, Clone, PartialEq)]
    struct PayjoinExtras {
        pj: Option<String>,
    }

    #[derive(Debug, Default)]
    struct PayjoinState {
        pj: Option<String>,
    }

    impl crate::de::DeserializationError for PayjoinExtras {
        type Error = core::convert::Infallible;
    }

    impl crate::de::DeserializeParams<'_> for PayjoinExtras {
        type DeserializationState = PayjoinState;
    }

    impl<'de> crate::de::DeserializationState<'de> for PayjoinState {
        type Value = PayjoinExtras;

        fn is_param_known(&self, key: &str) -> bool {
            key == "pj"
        }

        fn deserialize_temp(&mut self, key: &str, value: crate::Param<'_>) -> Result<crate::de::ParamKind, <Self::Value as crate::de::DeserializationError>::Error> {
            if key == "pj" {
                self.pj = Some(String::try_from(value).unwrap());
                Ok(crate::de::ParamKind::Known)
            } else {
                Ok(crate::de::ParamKind::Unknown)
            }
        }

        fn finalize(self) -> Result<Self::Value, <Self::Value as crate::de::DeserializationError>::Error> {
            Ok(PayjoinExtras {
                pj: self.pj,
            })
        }
    }

    impl<'a> crate::ser::SerializeParams for &'a PayjoinExtras {
        type Key = &'static str;
        type Value = alloc::string::String;
        type Iterator = alloc::vec::IntoIter<(Self::Key, Self::Value)>;

        fn serialize_params(self) -> Self::Iterator {
            let mut params = alloc::vec::Vec::new();
            if let Some(ref pj) = self.pj {
                params.push(("pj", pj.clone()));
            }
            params.into_iter()
        }
    }

    #[test]
    fn compose_two_extras() {
        // Test parsing with composed extras
        let input = "bitcoin:1andreas3batLhQa2FawWjeyjCqyBzypd?lightning=lnbc1&pj=https://example.com";
        let uri = input.parse::<Uri<'_, _, (LightningExtras, PayjoinExtras)>>()
            .unwrap()
            .require_network(bitcoin::Network::Bitcoin)
            .unwrap();
        
        assert_eq!(uri.address.to_string(), "1andreas3batLhQa2FawWjeyjCqyBzypd");
        assert_eq!(uri.extras.0.lightning, Some("lnbc1".to_string()));
        assert_eq!(uri.extras.1.pj, Some("https://example.com".to_string()));
    }

    #[test]
    fn compose_extras_serialization() {
        // Test serialization with composed extras
        let address: bitcoin::Address<bitcoin::address::NetworkUnchecked> = "1andreas3batLhQa2FawWjeyjCqyBzypd".parse().unwrap();
        let address = address.require_network(bitcoin::Network::Bitcoin).unwrap();
        let lightning_extras = LightningExtras {
            lightning: Some("lnbc1".to_string()),
        };
        let payjoin_extras = PayjoinExtras {
            pj: Some("https://example.com".to_string()),
        };
        
        let uri = Uri::with_extras(address, (lightning_extras, payjoin_extras));
        let uri_string = uri.to_string();
        
        assert!(uri_string.contains("lightning=lnbc1"));
        assert!(uri_string.contains("pj=https"));
    }

    #[test]
    fn compose_extras_with_no_extras() {
        // Test composing one extras with NoExtras
        let input = "bitcoin:1andreas3batLhQa2FawWjeyjCqyBzypd?lightning=lnbc1";
        let uri = input.parse::<Uri<'_, _, (LightningExtras, crate::NoExtras)>>()
            .unwrap()
            .require_network(bitcoin::Network::Bitcoin)
            .unwrap();
        
        assert_eq!(uri.address.to_string(), "1andreas3batLhQa2FawWjeyjCqyBzypd");
        assert_eq!(uri.extras.0.lightning, Some("lnbc1".to_string()));
    }

    #[test]
    fn compose_extras_required_param() {
        // Test that required params work with composed extras
        let input = "bitcoin:1andreas3batLhQa2FawWjeyjCqyBzypd?req-unknown=value";
        let result = input.parse::<Uri<'_, _, (LightningExtras, PayjoinExtras)>>();
        assert!(result.is_err());
    }

    #[test]
    fn compose_extras_optional_unknown_param() {
        // Test that optional unknown params are ignored with composed extras
        let input = "bitcoin:1andreas3batLhQa2FawWjeyjCqyBzypd?lightning=lnbc1&unknown=value";
        let uri = input.parse::<Uri<'_, _, (LightningExtras, PayjoinExtras)>>()
            .unwrap()
            .require_network(bitcoin::Network::Bitcoin)
            .unwrap();
        
        assert_eq!(uri.address.to_string(), "1andreas3batLhQa2FawWjeyjCqyBzypd");
        assert_eq!(uri.extras.0.lightning, Some("lnbc1".to_string()));
        assert_eq!(uri.extras.1.pj, None);
    }
}
