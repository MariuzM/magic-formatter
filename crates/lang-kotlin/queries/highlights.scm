(identifier) @variable

((identifier) @type
  (#match? @type "^[A-Z][A-Za-z0-9_]*[a-z]"))

(navigation_expression
  (identifier) @property .)

((identifier) @constant
  (#match? @constant "^[A-Z][A-Z0-9_]+$"))

(function_declaration
  name: (identifier) @function)

(call_expression
  (identifier) @function)

(call_expression
  (navigation_expression
    (identifier) @function.method .))

((call_expression
  (identifier) @type)
  (#match? @type "^[A-Z]"))

((call_expression
  (navigation_expression
    (identifier) @type .))
  (#match? @type "^[A-Z][A-Za-z0-9_]*[a-z]"))

(callable_reference
  (identifier) @function)

(parameter
  (identifier) @variable.parameter)

(class_parameter
  (identifier) @variable.parameter)

(lambda_parameters
  (variable_declaration
    (identifier) @variable.parameter))

(value_argument
  (identifier) @property
  .
  "=")

(user_type
  (identifier) @type)

(class_declaration
  name: (identifier) @type)

(object_declaration
  name: (identifier) @type)

(companion_object
  name: (identifier) @type)

(type_alias
  type: (identifier) @type)

(type_parameter
  (identifier) @type.parameter)

(enum_entry
  (identifier) @variant)

(annotation
  "@" @attribute)

(annotation
  (user_type
    (identifier) @attribute))

(annotation
  (constructor_invocation
    (user_type
      (identifier) @attribute)))

(file_annotation
  [
    "@"
    "file"
    ":"
  ] @attribute)

(file_annotation
  (constructor_invocation
    (user_type
      (identifier) @attribute)))

(package_header
  (qualified_identifier
    (identifier) @namespace))

(import
  (qualified_identifier
    (identifier) @namespace))

((import
  (qualified_identifier
    (identifier) @type .))
  (#match? @type "^[A-Z]"))

((import
  (qualified_identifier
    (identifier) @function .))
  (#match? @function "^[a-z_]"))

((labeled_expression
  (label) @_jump
  .
  (identifier) @label)
  (#match? @_jump "^(break|continue)@$"))

(label) @label

(navigation_expression
  "::"
  (identifier) @keyword
  (#eq? @keyword "class"))

(this_expression
  (identifier) @label)

(return_expression
  label: (identifier) @label)

((identifier) @boolean
  (#any-of? @boolean "true" "false"))

((identifier) @constant.builtin
  (#eq? @constant.builtin "null"))

((identifier) @keyword
  (#any-of? @keyword "break" "continue"))

[
  "package"
  "import"
  "class"
  "interface"
  "object"
  "fun"
  "val"
  "var"
  "typealias"
  "constructor"
  "init"
  "companion"
  "by"
  "where"
  "get"
  "set"
  "field"
  "property"
  "receiver"
  "param"
  "setparam"
  "delegate"
  "file"
  "dynamic"
  "if"
  "else"
  "when"
  "for"
  "while"
  "do"
  "try"
  "catch"
  "finally"
  "throw"
  "return"
  "return@"
  "in"
  "!in"
  "is"
  "!is"
  "as"
  "as?"
  "this"
  "this@"
  "super"
  "super@"
  "abstract"
  "actual"
  "annotation"
  "const"
  "crossinline"
  "data"
  "enum"
  "expect"
  "external"
  "final"
  "infix"
  "inline"
  "inner"
  "internal"
  "lateinit"
  "noinline"
  "open"
  "operator"
  "out"
  "override"
  "private"
  "protected"
  "public"
  "sealed"
  "suspend"
  "tailrec"
  "value"
  "vararg"
] @keyword

(reification_modifier) @keyword

[
  "="
  "+="
  "-="
  "*="
  "/="
  "%="
  "=="
  "!="
  "==="
  "!=="
  "<="
  ">="
  "&&"
  "||"
  "!"
  "!!"
  "+"
  "-"
  "*"
  "/"
  "%"
  "++"
  "--"
  "->"
  "?:"
  ".."
  "..<"
  "::"
] @operator

(binary_expression
  operator: [
    "<"
    ">"
  ] @operator)

[
  (number_literal)
  (float_literal)
] @number
