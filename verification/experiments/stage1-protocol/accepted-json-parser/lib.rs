#![forbid(unsafe_code)]

#[cfg(test)]
mod earlier {
    include!("../parser-binding/tests.rs");
}

#[cfg(test)]
mod fields;
