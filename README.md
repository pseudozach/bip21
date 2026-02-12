# Rust implementation of BIP21

Rust-idiomatic, compliant, flexible and performant BIP21 crate.

## About

**Important:** while lot of work went into polishing the crate it's still considered
early-development!

* Rust-idiomatic: uses strong types, standard traits and other things
* Compliant: implements all requirements of BIP21, including protections to not forget about
             `req-`. (But see features.)
* Flexible: enables parsing/serializing additional arguments not defined by BIP21.
* Performant: uses zero-copy deserialization and lazy evaluation wherever possible.

Serialization and deserialization is inspired by `serde` with these important differences:

* Deserialization signals if the field is known so that `req-` fields can be rejected.
* Much simpler API - we don't need all the features.
* Use of [`Param<'a>`] to enable lazy evaluation.

The crate is `no_std` but does require `alloc`.

## Composable Extras

Modern BIP21 usage often requires supporting multiple parameter extensions from different sources (e.g., Lightning Network invoices, Payjoin endpoints, Silent Payments). This crate supports composing multiple `Extras` implementations using tuples, allowing each parameter set to be implemented and maintained in its own crate:

```rust
// Define your extras types (or import from other crates)
type MyExtras = (LightningExtras, PayjoinExtras);

// Parse a URI with composed extras
let uri: Uri<'_, _, MyExtras> = uri_string.parse()?;

// Access parameters from each extras type
let lightning_invoice = uri.extras.0.lightning;
let payjoin_endpoint = uri.extras.1.pj;
```

This eliminates the need to duplicate deserialization/serialization logic and allows you to:
- Reuse existing, tested implementations from dependency crates
- Compose any combination of extras as needed for your application
- Avoid maintaining large monolithic extras implementations

For more details, see the [documentation](https://docs.rs/bip21).

## Features    

* `std` enables integration with `std` - mainly `std::error::Error`.
* `non-compliant-bytes` - enables use of non-compliant API that can parse non-UTF-8 URI values.

## MSRV

1.56.1

## License

MITNFA
