#![deny(missing_docs)]
#![deny(rustdoc::broken_intra_doc_links)]

//! This crate contains the types and functions for executing a Sunscreen circuit
//! (i.e. an [`Circuit`](sunscreen_circuit::Circuit)).

mod error;
mod keys;
mod metadata;
mod run;
mod runtime;

pub use crate::error::*;
pub use crate::keys::*;
pub use crate::metadata::*;
pub use run::*;
pub use runtime::*;

use seal::{Ciphertext as SealCiphertext, Plaintext as SealPlaintext};
use serde::Serialize;

#[derive(Debug, Serialize)]
/**
 * The underlying backend implementation of a plaintext (e.g. SEAL's [`Plaintext`](seal::Plaintext)).
 */
pub enum InnerPlaintext {
    /**
     * This plaintext wraps a SEAL [`Plaintext`](seal::Plaintext).
     */
    Seal(Vec<SealPlaintext>),
}

#[derive(Debug, Serialize)]
/**
 * Represents an encoded plaintext suitable for use in the underlying scheme.
 */
pub struct Plaintext {
    /**
     * The scheme and backend-specific plaintext.
     */
    pub inner: InnerPlaintext,
}

/**
 * The underlying backend implementation of a ciphertext (e.g SEAL's [`Ciphertext`](seal::Ciphertext)).
 */
pub enum InnerCiphertext {
    /**
     * A set of ciphertexts in SEAL's runtime.
     */
    Seal(Vec<SealCiphertext>),
}

/**
 * An encryption of the given data type. Note, the data type is stored in plaintext and is considered
 * part of Sunscreen's runtime protocol.
 */
pub struct Ciphertext {
    /**
     * The data type contained in this ciphertext. Note, this type metadata is stored in the clear.
     */
    pub data_type: Type,

    /**
     * The scheme and backend-specific plaintext.
     */
    pub inner: InnerCiphertext,
}

/**
 * This trait denotes one may attempt to turn this type into a plaintext.
 */
pub trait TryIntoPlaintext {
    /**
     * Attempts to turn this type into a [`Plaintext`].
     */
    fn try_into_plaintext(&self, params: &Params) -> Result<Plaintext>;
}

/**
 * This trait specifies one may attempt to convert a plaintext into this type.
 */
pub trait TryFromPlaintext
where
    Self: Sized,
{
    /**
     * Attempts to turn a [`Plaintext`] into `Self`. On success, returns
     */
    fn try_from_plaintext(plaintext: &Plaintext, params: &Params) -> Result<Self>;
}

/**
 * Declare how many ciphertexts an FheType decomposes into. The runtime needs this
 * to correctly bundle return values from a circuit.
 */
pub trait NumCiphertexts {
    /**
     * The number of ciphertexts this type decomposes into.
     */
    const NUM_CIPHERTEXTS: usize;
}

/**
 * Denotes the given rust type is an encoding in an FHE scheme
 */
pub trait FheType: TypeNameInstance + TryIntoPlaintext + TryFromPlaintext + NumCiphertexts {}

/**
 * Denotes the given type is valid under the BFV scheme.
 */
pub trait BfvType: FheType {}

/**
 * A trait the gives a name an version to a given type
 */
pub trait TypeName {
    /**
     * Returns the [`Type`] of the `&self`. Lives only on the instance so you can be object-safe
     * for use in `dyn TypeName`.
     */
    fn type_name() -> Type;
}

/**
 * A trait the gives a name an version to a given type
 */
pub trait TypeNameInstance {
    /**
     * Returns the [`Type`] of the `&self`. Lives only on the instance so you can be object-safe
     * for use in `dyn TypeName`.
     */
    fn type_name_instance(&self) -> Type;
}
