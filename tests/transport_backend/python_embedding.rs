//! Test-only bridge to the candidate Python codec.

use pyo3::{
    Py, Python,
    sync::PyOnceLock,
    types::{PyAnyMethods, PyBytes, PyModule},
};
use std::ffi::CString;

const BRIDGE: &str = include_str!("python_embedding.py");
static BRIDGE_MODULE: PyOnceLock<Py<PyModule>> = PyOnceLock::new();

pub(super) fn roundtrip(message_name: &str, bytes: &[u8]) -> Vec<u8> {
    Python::attach(|py| {
        let module = BRIDGE_MODULE.get_or_init(py, || {
            let code = CString::new(BRIDGE).expect("Python bridge source has no NUL");
            PyModule::from_code(
                py,
                code.as_c_str(),
                c"gwz-core/tests/transport_backend/python_embedding.py",
                c"gwz_test_python_embedding",
            )
            .unwrap_or_else(|error| panic!("load embedded Python codec bridge: {error}"))
            .unbind()
        });
        let roundtrip = module
            .bind(py)
            .getattr("roundtrip")
            .unwrap_or_else(|error| panic!("find Python codec roundtrip: {error}"));
        let result = roundtrip
            .call1((
                message_name,
                PyBytes::new(py, bytes),
                env!("CARGO_MANIFEST_DIR"),
            ))
            .unwrap_or_else(|error| panic!("candidate Python codec roundtrip: {error}"));
        result
            .extract::<Vec<u8>>()
            .unwrap_or_else(|error| panic!("extract candidate Python codec bytes: {error}"))
    })
}
