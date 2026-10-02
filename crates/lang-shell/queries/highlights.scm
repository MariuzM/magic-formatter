(comment) @comment

[
  (string)
  (raw_string)
  (ansi_c_string)
  (translated_string)
  (heredoc_body)
  (heredoc_start)
  (heredoc_end)
] @string

[
  (number)
  (file_descriptor)
] @number

((command_name
  (word) @function.builtin)
  (#any-of? @function.builtin
    "alias" "bg" "bind" "break" "builtin" "caller" "cd" "command" "compgen" "complete" "continue" "dirs" "disown"
    "echo" "enable" "eval" "exec" "exit" "false" "fc" "fg" "getopts" "hash" "help" "history" "jobs" "kill" "let"
    "logout" "mapfile" "popd" "printf" "pushd" "pwd" "read" "readarray" "return" "set" "shift" "shopt" "source"
    "suspend" "test" "times" "trap" "true" "type" "ulimit" "umask" "unalias" "wait"))

(command_name
  (word) @function)

(function_definition
  name: (word) @function)

[
  (variable_name)
  (special_variable_name)
] @variable

(simple_expansion
  "$" @variable)

(expansion
  [
    "${"
    "}"
  ] @variable)

((word) @variable.parameter
  (#match? @variable.parameter "^-"))

[
  "if"
  "then"
  "else"
  "elif"
  "fi"
  "case"
  "esac"
  "in"
  "for"
  "select"
  "while"
  "until"
  "do"
  "done"
  "function"
  "declare"
  "local"
  "export"
  "readonly"
  "typeset"
  "unset"
  "unsetenv"
] @keyword

(test_operator) @operator

[
  "&&"
  "||"
  "|"
  "|&"
  "!"
  ">"
  ">>"
  "<"
  "<<"
  "<<-"
  "<<<"
  "&>"
  "&>>"
  ">&"
  "<&"
  ">|"
  "="
  "+="
  "=="
  "!="
  "=~"
  ";;"
  ";&"
  ";;&"
] @operator
