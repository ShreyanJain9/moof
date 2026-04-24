// Bignum backing for arbitrary-precision integers.
//
// This is the OVERFLOW STORAGE for the one moof Integer type. Users
// never see it distinctly — its prototype is type_protos[PROTO_INT],
// so dispatch finds the same Integer handlers that primitive i48
// values do, and `typeName` answers `'Integer` either way.
//
// Construction goes through `Heap::alloc_integer(BigInt)` which
// demotes to a NaN-boxed i48 if the value fits, allocating a foreign
// object only when it doesn't. Integer arithmetic in the numeric
// plugin uses checked ops and promotes on overflow; every result
// round-trips through alloc_integer so small results always come
// back as primitives.

use crate::foreign::ForeignType;

#[derive(Clone, Debug)]
pub struct BigInt(pub num_bigint::BigInt);

impl ForeignType for BigInt {
    fn type_name() -> &'static str { "moof.core.BigInt" }
    // prototype_name returns "Integer" so cross-vat copy and the
    // built-in registration both wire BigInt to the one Integer proto.
    fn prototype_name() -> &'static str { "Integer" }

    fn serialize(&self) -> Vec<u8> {
        // signed bytes-be encoding via num-bigint. stable and round-trips.
        self.0.to_signed_bytes_be()
    }
    fn deserialize(bytes: &[u8]) -> Result<Self, String> {
        Ok(BigInt(num_bigint::BigInt::from_signed_bytes_be(bytes)))
    }

    fn equal(&self, other: &Self) -> bool { self.0 == other.0 }

    fn describe(&self) -> String { self.0.to_string() }
}
