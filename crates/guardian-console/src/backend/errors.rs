//! Server errors in words a teacher understands.

use tonic::{Code, Status};

/// The text shown for a failed call.
pub fn status_text(s: &Status) -> String {
    match s.code() {
        Code::Unauthenticated => "Your session has ended. Sign in again.".into(),
        Code::PermissionDenied => "You are not allowed to do that.".into(),
        Code::InvalidArgument | Code::FailedPrecondition | Code::NotFound | Code::AlreadyExists => {
            let m = s.message().trim();
            if m.is_empty() {
                format!("{:?}", s.code())
            } else {
                m.to_string()
            }
        }
        Code::Unavailable => format!("Cannot reach the server: {}", s.message()),
        _ => format!("Server error: {}", s.message()),
    }
}

/// For `Login`/`Init`, a wrong password is `UNAUTHENTICATED` too: show the
/// server's own message there.
pub fn login_error_text(s: &Status) -> String {
    match s.code() {
        Code::Unauthenticated | Code::PermissionDenied | Code::ResourceExhausted => {
            let m = s.message().trim();
            if m.is_empty() {
                "Wrong login or password.".into()
            } else {
                m.to_string()
            }
        }
        _ => status_text(s),
    }
}
