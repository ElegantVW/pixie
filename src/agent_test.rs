#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_turnout_success() {
        let result = TurnOut { output: "success".to_string(), error: None };
        assert!(result.error.is_none());
    }

    #[test]
    fn test_turnout_error() {
        let result = TurnOut { output: "error".to_string(), error: Some("details".to_string()) };
        assert!(result.error.is_some());
    }
}