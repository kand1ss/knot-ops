use knot_core::errors::ClientError;
use knot_terminal::ErrorReport;

pub trait ErrorReportExt {
    fn from_error(error: ClientError) -> ErrorReport;
}

impl ErrorReportExt for ErrorReport {
    fn from_error(error: ClientError) -> ErrorReport {
        let mut report = ErrorReport::new(error.to_string());
        if let Some(solution) = error.solution() {
            report = report.with_solution(solution);
        };
        if let Some(context) = error.context() {
            report = report.with_context(context);
        };
        report
    }
}
