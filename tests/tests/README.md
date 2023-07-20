# Openraft integration tests

The integration tests for Openraft are stored in this directory and rely on
`memstore` with `serde` enabled.
Certain tests in Openraft require the `serde` feature to be disabled.
To avoid enabling `serde` for all tests in Openraft, we must relocate the
integration tests to a separate crate.


## Case naming convention

A test file name starts with `t[\d\d]_`, where `\d\d` is the test case number indicating priority.

- `t00`: not used.
- `t10`: basic behaviors.
- `t20`: life cycle test cases. 
- `t30`: special cases for an API. 
- `t40`: not used. 
- `t50`: environment depended behaviors.  
- `t60`: config related behaviors. 
- `t70`: not used.
- `t80`: not used.
- `t90`: issue fixes. 
