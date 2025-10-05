/// Used in engine callbacks.
///
/// Sends termination signal to the main event loop and returns false if $result is an error.
#[macro_export]
macro_rules! error_in_callback {
  ($state:ident, $result:expr) => {
    error_in_callback!($state, $result, return false)
  };

  ($state:ident, $result:expr, return $return_value:expr) => {
    match $result {
      Ok(v) => v,
      Err(e) => {
        let _ = $state
          .terminate
          .unbounded_send(::anyhow::Result::Err(::anyhow::Error::from(e)));
        return $return_value;
      }
    }
  };
}
