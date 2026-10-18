use crate::function::{Clamper, Function, TupleVec, Values, interpolate};
use hayro_syntax::object::Array;
use hayro_syntax::object::Dict;
use hayro_syntax::object::Object;
use hayro_syntax::object::dict::keys::{BOUNDS, ENCODE, FUNCTIONS};
use smallvec::smallvec;

/// A type 3 function (stitching function).
#[derive(Debug)]
pub(crate) struct Type3 {
    functions: Vec<Function>,
    bounds: Vec<f32>,
    encode: TupleVec,
    clamper: Clamper,
}

impl Type3 {
    /// Create a new type 3 function.
    pub(crate) fn new(dict: &Dict<'_>) -> Option<Self> {
        let clamper = Clamper::new(dict)?;

        let functions = dict
            .get::<Array<'_>>(FUNCTIONS)
            .and_then(|d| d.iter::<Object<'_>>().map(|o| Function::new(&o)).collect())?;
        let domain = *clamper.domain.first()?;
        let mut bounds = vec![domain.0 - 0.0001];
        if let Some(a) = dict.get::<Array<'_>>(BOUNDS) {
            bounds.extend(a.iter::<f32>());
        }
        // Add a small delta so that the interval is considered to be closed on the right.
        bounds.push(domain.1 + 0.0001);

        let encode = dict.get::<TupleVec>(ENCODE)?;

        Some(Self {
            functions,
            clamper,
            bounds,
            encode,
        })
    }

    /// Evaluate the function with the given input.
    pub(crate) fn eval(&self, input: f32) -> Option<Values> {
        let mut input = [input];
        self.clamper.clamp_input(&mut input);

        let index = find_interval(&self.bounds, input[0])?;

        let bounds_i = *self.bounds.get(index + 1)?;
        let bounds_i_minus_1 = *self.bounds.get(index)?;

        // - 1 because we inserted a dummy bound in the constructor.
        let encoding = self.encode.get(index)?;
        let function = self.functions.get(index)?;
        let encoded = interpolate(input[0], bounds_i_minus_1, bounds_i, encoding.0, encoding.1);

        let mut evaluated = function.eval(smallvec![encoded])?;

        self.clamper.clamp_output(&mut evaluated);

        Some(evaluated)
    }
}

fn find_interval(bounds: &[f32], x: f32) -> Option<usize> {
    if x < *bounds.first()? || x >= *bounds.last()? {
        return None;
    }

    match bounds.binary_search_by(|val| {
        if *val <= x {
            std::cmp::Ordering::Less
        } else {
            std::cmp::Ordering::Greater
        }
    }) {
        Ok(i) => Some(i - 1),
        Err(i) => Some(i - 1),
    }
}
