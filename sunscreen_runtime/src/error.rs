use crate::Type;

#[derive(Debug, Clone, PartialEq)]
/**
 * Represents an error that can occur in this crate.
 */
pub enum Error {
    /**
     * An issue with an [`Circuit`](sunscreen_circuit::Circuit).
     */
    IRError(sunscreen_circuit::Error),

    /**
     * An error occurred in the SEAL library.
     */
    SealError(seal::Error),

    /**
     * Tried to run a circuit that requires relinearization keys, but didn't provide any.
     */
    MissingRelinearizationKeys,

    /**
     * Tried to run a circuit that requires Galois keys, but didn't provide any.
     */
    MissingGaloisKeys,

    /**
     * Returned when:
     * * The wrong number of ciphertexts were provided as parameters to a circuit.
     * * The wrong number of ciphertexts were returned from a circuit.
     */
    IncorrectCiphertextCount,

    /**
     * An argument is incompatible with the parameters in the runtime.
     */
    ParameterMismatch,

    /**
     * The given arguments do not match the call signature of the circuit.
     */
    ArgumentMismatch {
        /**
         * The arguments in the call signature of the circuit.
         */
        expected: Vec<Type>,

        /**
         * The given arguments.
         */
        actual: Vec<Type>,
    },

    /**
     * The given return types do not match the circuit interface.
     */
    ReturnMismatch {
        /**
         * The return types in the call signature of the circuit.
         */
        expected: Vec<Type>,

        /**
         * The given return types.
         */
        actual: Vec<Type>,
    },
}

impl From<sunscreen_circuit::Error> for Error {
    fn from(err: sunscreen_circuit::Error) -> Self {
        Self::IRError(err)
    }
}

impl From<seal::Error> for Error {
    fn from(err: seal::Error) -> Self {
        Self::SealError(err)
    }
}

/**
 * Wrapper around [`Result`](std::result::Result) with this crate's error type.
 */
pub type Result<T> = std::result::Result<T, Error>;
