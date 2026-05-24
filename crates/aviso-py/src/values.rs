//! `PyO3` value types: `Notification`, `NotifyResponse`, `SchemaCatalog`,
//! `SchemaResponse`, and the supporting helpers.
//!
//! These classes carry the same logical data as their `aviso` core
//! counterparts but hold their fields directly instead of wrapping the
//! core structs. The core's structs are `#[non_exhaustive]` and cannot be
//! constructed from outside the core crate; the wrappers stay
//! self-contained so the Python side can build instances for tests, for
//! deserialisation from external JSON, and for receiving live events
//! once the watch supervisor is wired.
//!
//! Payload fields that hold JSON values are deep-converted to native
//! Python objects via `pythonize` so callers can `print(n.payload)` and
//! see a `dict`/`list`/scalar rather than a `serde_json::Value` wrapper.
//!
//! Every class implements `__repr__` and `__eq__`. None of them implement
//! `__hash__`: their payload fields hold mutable Python objects after
//! conversion, and a value type that contains unhashable inner values is
//! itself unhashable per Python convention. Each class sets
//! `__hash__ = None` so a misuse like `set([notification])` raises
//! `TypeError` loudly rather than falling back to an id-based hash.

use std::collections::BTreeMap;

use aviso::{Notification, NotifyResponse, SchemaCatalog, SchemaResponse, StreamSchema};
use pyo3::prelude::*;
use pyo3::types::PyDict;
use pythonize::pythonize;

/// `PyO3` wrapper carrying a received notification's fields.
#[pyclass(
    name = "Notification",
    module = "aviso._native",
    frozen,
    eq,
    skip_from_py_object
)]
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct PyNotification {
    event_type: String,
    sequence: u64,
    identifier: BTreeMap<String, String>,
    payload: serde_json::Value,
    cloudevent: Option<serde_json::Value>,
}

#[pymethods]
impl PyNotification {
    #[new]
    #[pyo3(signature = (event_type, sequence, identifier, payload, cloudevent = None))]
    fn new(
        py: Python<'_>,
        event_type: String,
        sequence: u64,
        identifier: BTreeMap<String, String>,
        payload: &Bound<'_, PyAny>,
        cloudevent: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Self> {
        let _ = py;
        let payload_value: serde_json::Value = pythonize::depythonize(payload)?;
        let cloudevent_value = match cloudevent {
            Some(obj) => Some(pythonize::depythonize(obj)?),
            None => None,
        };
        Ok(Self {
            event_type,
            sequence,
            identifier,
            payload: payload_value,
            cloudevent: cloudevent_value,
        })
    }

    #[getter]
    fn event_type(&self) -> &str {
        &self.event_type
    }

    #[getter]
    fn sequence(&self) -> u64 {
        self.sequence
    }

    #[getter]
    fn identifier<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let dict = PyDict::new(py);
        for (k, v) in &self.identifier {
            dict.set_item(k, v)?;
        }
        Ok(dict)
    }

    #[getter]
    fn payload<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        pythonize(py, &self.payload).map_err(Into::into)
    }

    #[getter]
    fn cloudevent<'py>(&self, py: Python<'py>) -> PyResult<Option<Bound<'py, PyAny>>> {
        match &self.cloudevent {
            Some(value) => Ok(Some(pythonize(py, value)?)),
            None => Ok(None),
        }
    }

    fn as_dict<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let dict = PyDict::new(py);
        dict.set_item("event_type", &self.event_type)?;
        dict.set_item("sequence", self.sequence)?;
        dict.set_item("identifier", self.identifier(py)?)?;
        dict.set_item("payload", self.payload(py)?)?;
        Ok(dict)
    }

    fn __repr__(&self) -> String {
        format!(
            "Notification(event_type={:?}, sequence={}, identifier={:?})",
            self.event_type, self.sequence, self.identifier
        )
    }

    #[classattr]
    const __hash__: Option<Py<PyAny>> = None;
}

impl PyNotification {
    pub(crate) fn from_core(notification: Notification) -> Self {
        Self {
            event_type: notification.event_type,
            sequence: notification.sequence,
            identifier: notification.identifier,
            payload: notification.payload,
            cloudevent: notification.cloudevent,
        }
    }
}

/// `PyO3` wrapper for a successful publish response.
#[pyclass(
    name = "NotifyResponse",
    module = "aviso._native",
    frozen,
    eq,
    skip_from_py_object
)]
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct PyNotifyResponse {
    status: String,
    request_id: String,
    processed_at: String,
}

#[pymethods]
impl PyNotifyResponse {
    #[new]
    fn new(status: String, request_id: String, processed_at: String) -> Self {
        Self {
            status,
            request_id,
            processed_at,
        }
    }

    #[getter]
    fn status(&self) -> &str {
        &self.status
    }

    #[getter]
    fn request_id(&self) -> &str {
        &self.request_id
    }

    #[getter]
    fn processed_at(&self) -> &str {
        &self.processed_at
    }

    fn as_dict<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let dict = PyDict::new(py);
        dict.set_item("status", &self.status)?;
        dict.set_item("request_id", &self.request_id)?;
        dict.set_item("processed_at", &self.processed_at)?;
        Ok(dict)
    }

    fn __repr__(&self) -> String {
        format!(
            "NotifyResponse(status={:?}, request_id={:?})",
            self.status, self.request_id
        )
    }

    #[classattr]
    const __hash__: Option<Py<PyAny>> = None;
}

impl PyNotifyResponse {
    pub(crate) fn from_core(response: NotifyResponse) -> Self {
        Self {
            status: response.status,
            request_id: response.request_id,
            processed_at: response.processed_at,
        }
    }
}

/// `PyO3` wrapper for the full schema catalog.
#[pyclass(
    name = "SchemaCatalog",
    module = "aviso._native",
    frozen,
    eq,
    skip_from_py_object
)]
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct PySchemaCatalog {
    status: String,
    schema: BTreeMap<String, OwnedSchema>,
    event_types: Vec<String>,
    total_schemas: u32,
}

#[derive(Clone, PartialEq, Eq)]
struct OwnedSchema {
    payload: Option<serde_json::Value>,
    identifier: BTreeMap<String, serde_json::Value>,
}

impl From<StreamSchema> for OwnedSchema {
    fn from(schema: StreamSchema) -> Self {
        Self {
            payload: schema.payload,
            identifier: schema.identifier,
        }
    }
}

#[pymethods]
impl PySchemaCatalog {
    #[getter]
    fn status(&self) -> &str {
        &self.status
    }

    #[getter]
    fn event_types(&self) -> Vec<String> {
        self.event_types.clone()
    }

    #[getter]
    fn total_schemas(&self) -> u32 {
        self.total_schemas
    }

    #[getter]
    fn schema<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let dict = PyDict::new(py);
        for (event_type, schema) in &self.schema {
            dict.set_item(event_type, owned_schema_to_dict(py, schema)?)?;
        }
        Ok(dict)
    }

    fn as_dict<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let dict = PyDict::new(py);
        dict.set_item("status", &self.status)?;
        dict.set_item("schema", self.schema(py)?)?;
        dict.set_item("event_types", self.event_types())?;
        dict.set_item("total_schemas", self.total_schemas)?;
        Ok(dict)
    }

    fn __repr__(&self) -> String {
        format!(
            "SchemaCatalog(status={:?}, total_schemas={}, event_types={:?})",
            self.status, self.total_schemas, self.event_types
        )
    }

    #[classattr]
    const __hash__: Option<Py<PyAny>> = None;
}

impl PySchemaCatalog {
    pub(crate) fn from_core(catalog: SchemaCatalog) -> Self {
        Self {
            status: catalog.status,
            schema: catalog
                .schema
                .into_iter()
                .map(|(k, v)| (k, v.into()))
                .collect(),
            event_types: catalog.event_types,
            total_schemas: catalog.total_schemas,
        }
    }
}

/// `PyO3` wrapper for a single schema response.
#[pyclass(
    name = "SchemaResponse",
    module = "aviso._native",
    frozen,
    eq,
    skip_from_py_object
)]
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct PySchemaResponse {
    status: String,
    event_type: String,
    schema: OwnedSchema,
}

#[pymethods]
impl PySchemaResponse {
    #[getter]
    fn status(&self) -> &str {
        &self.status
    }

    #[getter]
    fn event_type(&self) -> &str {
        &self.event_type
    }

    #[getter]
    fn schema<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        owned_schema_to_dict(py, &self.schema)
    }

    fn as_dict<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let dict = PyDict::new(py);
        dict.set_item("status", &self.status)?;
        dict.set_item("event_type", &self.event_type)?;
        dict.set_item("schema", self.schema(py)?)?;
        Ok(dict)
    }

    fn __repr__(&self) -> String {
        format!(
            "SchemaResponse(event_type={:?}, status={:?})",
            self.event_type, self.status
        )
    }

    #[classattr]
    const __hash__: Option<Py<PyAny>> = None;
}

impl PySchemaResponse {
    pub(crate) fn from_core(response: SchemaResponse) -> Self {
        Self {
            status: response.status,
            event_type: response.event_type,
            schema: response.schema.into(),
        }
    }
}

fn owned_schema_to_dict<'py>(
    py: Python<'py>,
    schema: &OwnedSchema,
) -> PyResult<Bound<'py, PyDict>> {
    let dict = PyDict::new(py);
    match &schema.payload {
        Some(value) => dict.set_item("payload", pythonize(py, value)?)?,
        None => dict.set_item("payload", py.None())?,
    }
    let ident = PyDict::new(py);
    for (k, v) in &schema.identifier {
        ident.set_item(k, pythonize(py, v)?)?;
    }
    dict.set_item("identifier", ident)?;
    Ok(dict)
}

pub(crate) fn register_value_types(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyNotification>()?;
    m.add_class::<PyNotifyResponse>()?;
    m.add_class::<PySchemaCatalog>()?;
    m.add_class::<PySchemaResponse>()?;
    Ok(())
}
