// Copyright (C) 2020-2021 Andy Kurnia.

// https://github.com/kkawakam/rustyline/blob/master/examples/example.rs

#[derive(rustyline_derive::Helper)]
pub struct MyHelper {
    completer: rustyline::completion::FilenameCompleter,
    highlighter: rustyline::highlight::MatchingBracketHighlighter,
    validator: rustyline::validate::MatchingBracketValidator,
    hinter: rustyline::hint::HistoryHinter,
    colored_prompt: String,
}

impl rustyline::completion::Completer for MyHelper {
    type Candidate = rustyline::completion::Pair;

    fn complete(
        &self,
        line: &str,
        pos: usize,
        ctx: &rustyline::Context<'_>,
    ) -> Result<(usize, Vec<rustyline::completion::Pair>), rustyline::error::ReadlineError> {
        self.completer.complete(line, pos, ctx)
    }
}

impl rustyline::hint::Hinter for MyHelper {
    type Hint = String;

    fn hint(&self, line: &str, pos: usize, ctx: &rustyline::Context<'_>) -> Option<String> {
        self.hinter.hint(line, pos, ctx)
    }
}

impl rustyline::highlight::Highlighter for MyHelper {
    fn highlight_prompt<'b, 's: 'b, 'p: 'b>(
        &'s self,
        prompt: &'p str,
        default: bool,
    ) -> std::borrow::Cow<'b, str> {
        if default {
            std::borrow::Cow::Borrowed(&self.colored_prompt)
        } else {
            std::borrow::Cow::Borrowed(prompt)
        }
    }

    fn highlight_hint<'h>(&self, hint: &'h str) -> std::borrow::Cow<'h, str> {
        std::borrow::Cow::Owned("\x1b[1m".to_owned() + hint + "\x1b[m")
    }

    fn highlight<'l>(&self, line: &'l str, pos: usize) -> std::borrow::Cow<'l, str> {
        self.highlighter.highlight(line, pos)
    }

    fn highlight_char(&self, line: &str, pos: usize) -> bool {
        self.highlighter.highlight_char(line, pos)
    }
}

impl rustyline::validate::Validator for MyHelper {
    fn validate(
        &self,
        ctx: &mut rustyline::validate::ValidationContext<'_>,
    ) -> rustyline::Result<rustyline::validate::ValidationResult> {
        self.validator.validate(ctx)
    }

    fn validate_while_typing(&self) -> bool {
        self.validator.validate_while_typing()
    }
}

pub fn new_rl_editor() -> rustyline::Editor<MyHelper> {
    let mut rl = rustyline::Editor::new();
    rl.set_helper(Some(MyHelper {
        completer: rustyline::completion::FilenameCompleter::new(),
        highlighter: rustyline::highlight::MatchingBracketHighlighter::new(),
        hinter: rustyline::hint::HistoryHinter {},
        colored_prompt: ">> ".to_owned(),
        validator: rustyline::validate::MatchingBracketValidator::new(),
    }));
    rl
}
