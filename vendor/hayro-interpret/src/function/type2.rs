use crate::function::{Clamper, Values};
use hayro_syntax::object::Dict;
use hayro_syntax::object::Number;
use hayro_syntax::object::dict::keys::{C0, C1, N};
use smallvec::{SmallVec, smallvec};

/// A type 2 function (exponential function).
#[derive(Debug)]
pub(crate) struct Type2 {
    c0: Values,
    c1: Values,
    clamper: Clamper,
    n: f32,
}

impl Type2 {
    /// Create a new type 2 function.
    pub(crate) fn new(dict: &Dict<'_>) -> Option<Self> {
        let c0 = dict.get::<Values>(C0).unwrap_or(smallvec![0.0]);
        let c1 = dict.get::<Values>(C1).unwrap_or(smallvec![1.0]);
        let clamper = Clamper::new(dict)?;
        let n = dict.get::<Number>(N)?.as_f64() as f32;

        Some(Self { c0, c1, clamper, n })
    }

    /// Evaluate the function with the given input.
    pub(crate) fn eval(&self, input: f32) -> Values {
        let mut input = [input];
        self.clamper.clamp_input(&mut input);

        let mut out = self
            .c0
            .iter()
            .zip(self.c1.iter())
            .map(|(c0, c1)| *c0 + input[0].powf(self.n) * (*c1 - *c0))
            .collect::<SmallVec<_>>();

        self.clamper.clamp_output(&mut out);

        out
    }
}
