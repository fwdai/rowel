//! The command line as one string. A grant in `env` mode names the *full*
//! command line (`spec/03`), so every party that writes one — `rowel run`
//! asking for the grant and resolving under it, the hook rewriting a tool
//! call, `rowel_request_secret` telling the agent what to ask for — has to
//! produce the same bytes for the same words. [`line`] is that rule, and
//! [`words`] reads a line a shell would run back into words, for the hook.

/// The words joined by one space, each quoted by [`quote`].
pub fn line(argv: &[String]) -> String {
    argv.iter()
        .map(|word| quote(word))
        .collect::<Vec<_>>()
        .join(" ")
}

/// `word` as a shell reads it back: bare when nothing in it means anything
/// to a shell, else in single quotes, with an embedded quote written `'\''`.
/// Single quotes are chosen because inside them nothing is special, so the
/// rule has one case and a line is readable in the grant dialog.
pub fn quote(word: &str) -> String {
    let plain = !word.is_empty()
        && !word
            .chars()
            .any(|c| c.is_whitespace() || "'\"$`\\*?|&;<>()#{}[]~".contains(c));
    if plain {
        return word.to_string();
    }
    let mut quoted = String::with_capacity(word.len() + 2);
    quoted.push('\'');
    for c in word.chars() {
        if c == '\'' {
            quoted.push_str("'\\''");
        } else {
            quoted.push(c);
        }
    }
    quoted.push('\'');
    quoted
}

/// The words of `text` as a POSIX shell would split them: unquoted
/// whitespace separates, single quotes keep everything, double quotes keep
/// everything but `\` before `"`, `\`, `$` or a backtick, and a backslash
/// outside quotes keeps the next character. `None` for a quote left open.
/// Only splitting: no expansion of any kind happens here, and whether the
/// line *asked* for one is the hook's own check ([`needs_shell`]).
pub fn words(text: &str) -> Option<Vec<String>> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut in_word = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\'' => {
                in_word = true;
                loop {
                    match chars.next()? {
                        '\'' => break,
                        other => word.push(other),
                    }
                }
            }
            '"' => {
                in_word = true;
                loop {
                    match chars.next()? {
                        '"' => break,
                        '\\' => {
                            let next = chars.next()?;
                            if !"\"\\$`".contains(next) {
                                word.push('\\');
                            }
                            word.push(next);
                        }
                        other => word.push(other),
                    }
                }
            }
            '\\' => {
                in_word = true;
                word.push(chars.next()?);
            }
            c if c.is_whitespace() => {
                if in_word {
                    words.push(std::mem::take(&mut word));
                    in_word = false;
                }
            }
            other => {
                in_word = true;
                word.push(other);
            }
        }
    }
    if in_word {
        words.push(word);
    }
    Some(words)
}

/// Whether a shell would do more with `text` than split it into words
/// ([`words`]): an operator (a pipe, `&`, `;`, a newline, a redirection,
/// parentheses) or an expansion (`$`, a backtick, a glob `*` `?` `[`, a `~`,
/// braces, a `#` comment) anywhere it is not quoted — and inside double
/// quotes, `$` and a backtick still expand. The hook leaves such a line
/// alone: rewriting it word by word would change what it runs, and left as
/// it is, it runs with the literal placeholder and fails, never with a value.
pub fn needs_shell(text: &str) -> bool {
    let (mut single, mut double, mut escaped) = (false, false, false);
    for c in text.chars() {
        if escaped {
            escaped = false;
        } else if single {
            single = c != '\'';
        } else if double {
            match c {
                '\\' => escaped = true,
                '"' => double = false,
                '$' | '`' => return true,
                _ => {}
            }
        } else {
            match c {
                '\\' => escaped = true,
                '\'' => single = true,
                '"' => double = true,
                '|' | '&' | ';' | '\n' | '<' | '>' | '(' | ')' | '$' | '`' | '*' | '?' | '['
                | ']' | '{' | '}' | '~' | '#' => return true,
                _ => {}
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(words: &[&str]) -> Vec<String> {
        words.iter().map(|w| w.to_string()).collect()
    }

    #[test]
    fn plain_words_stay_bare_and_the_rest_are_single_quoted() {
        assert_eq!(
            line(&argv(&["stripe", "balance", "retrieve", "--live"])),
            "stripe balance retrieve --live"
        );
        assert_eq!(
            line(&argv(&["sh", "-c", "echo $STRIPE_KEY"])),
            "sh -c 'echo $STRIPE_KEY'"
        );
        assert_eq!(quote("it's"), "'it'\\''s'");
        assert_eq!(quote(""), "''");
        assert_eq!(quote("a b"), "'a b'");
        assert_eq!(quote("path/to/file.txt"), "path/to/file.txt");
        assert_eq!(quote("--flag=value"), "--flag=value");
        for special in [
            "*", "?", "a|b", "a&b", "a;b", "<", ">", "(", ")", "`x`", "\\", "\"q\"", "#12",
            "{a,b}", "[x]", "~/x",
        ] {
            assert!(quote(special).starts_with('\''), "{special}");
        }
    }

    #[test]
    fn a_line_reads_back_into_the_words_it_was_made_from() {
        for words in [
            vec!["a", "b c", "it's", "$HOME", "", "*.rs", "x\"y"],
            vec!["sh", "-c", "echo $STRIPE_KEY | cat"],
        ] {
            let argv = argv(&words);
            assert_eq!(super::words(&line(&argv)).unwrap(), argv);
        }
    }

    #[test]
    fn words_split_as_a_shell_does() {
        assert_eq!(
            words("a  'b c' \"d e\" f\\ g").unwrap(),
            ["a", "b c", "d e", "f g"]
        );
        assert_eq!(
            words(r#""say \"hi\" \$x" plain"#).unwrap(),
            ["say \"hi\" $x", "plain"]
        );
        assert_eq!(words("'' x").unwrap(), ["", "x"]);
        assert_eq!(words("   ").unwrap(), Vec::<String>::new());
        assert_eq!(words("'open"), None);
        assert_eq!(words("\"open"), None);
        assert_eq!(words("trailing\\"), None);
    }

    #[test]
    fn what_a_shell_would_do_more_than_split_is_seen() {
        for line in [
            "a | b",
            "a && b",
            "a || b",
            "a; b",
            "a\nb",
            "a > f",
            "a < f",
            "a 2>&1",
            "echo `id`",
            "echo $(id)",
            "echo ${HOME}",
            "(a)",
            "\"$(id)\"",
            "echo $HOME",
            "echo \"$HOME\"",
            "ls *.rs",
            "cat ~/x",
            "echo {a,b}",
            "git commit -m fix #12",
            "echo \"it's\"; touch x",
        ] {
            assert!(needs_shell(line), "{line:?}");
        }
        for line in [
            "a b",
            "sh -c 'echo $X | cat'",
            "echo 'a;b'",
            "echo \\;",
            "--flag=value",
            "git commit -m \"#12\"",
            "echo \"it's; fine\"",
            "echo '*' \\*",
        ] {
            assert!(!needs_shell(line), "{line:?}");
        }
    }
}
