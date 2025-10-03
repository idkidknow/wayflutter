use thiserror::Error;
use crate::ffi;

#[derive(Debug, Error)]
pub enum FlutterEngineError {
    #[error("Invalid library version")]
    InvalidLibraryVersion,
    #[error("Invalid arguments")]
    InvalidArguments,
    #[error("Internal inconsistency")]
    InternalInconsistency,
    #[error("Unknown error: {0}")]
    Unknown(core::ffi::c_uint),
}

pub type FlutterEngineResult = Result<(), FlutterEngineError>;

pub trait FFIFlutterEngineResultExt {
    fn into_flutter_engine_result(self) -> FlutterEngineResult;
}

impl FFIFlutterEngineResultExt for ffi::FlutterEngineResult {
    fn into_flutter_engine_result(self) -> FlutterEngineResult {
        match self {
            ffi::FlutterEngineResult_kSuccess => Ok(()),
            ffi::FlutterEngineResult_kInvalidLibraryVersion => Err(FlutterEngineError::InvalidLibraryVersion),
            ffi::FlutterEngineResult_kInvalidArguments => Err(FlutterEngineError::InvalidArguments),
            ffi::FlutterEngineResult_kInternalInconsistency => Err(FlutterEngineError::InternalInconsistency),
            unknown => Err(FlutterEngineError::Unknown(unknown)),
        }
    }
}
