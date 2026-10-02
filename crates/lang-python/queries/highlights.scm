(identifier) @variable

((identifier) @type
  (#match? @type "^[A-Z]"))

((identifier) @constant
  (#match? @constant "^[A-Z][A-Z0-9_]*$"))

(parameters
  (identifier) @variable.parameter)

(parameters
  (list_splat_pattern
    (identifier) @variable.parameter))

(parameters
  (dictionary_splat_pattern
    (identifier) @variable.parameter))

(typed_parameter
  (identifier) @variable.parameter)

(typed_parameter
  (list_splat_pattern
    (identifier) @variable.parameter))

(typed_parameter
  (dictionary_splat_pattern
    (identifier) @variable.parameter))

(default_parameter
  name: (identifier) @variable.parameter)

(typed_default_parameter
  name: (identifier) @variable.parameter)

(lambda_parameters
  (identifier) @variable.parameter)

(keyword_argument
  name: (identifier) @variable.parameter)

((identifier) @type.builtin
  (#match? @type.builtin "^(int|str|float|bool|bytes|bytearray|complex|dict|list|set|frozenset|tuple|object)$"))

(keyword_pattern
  .
  (identifier) @property)

(attribute
  attribute: (identifier) @property)

((attribute
  attribute: (identifier) @type)
  (#match? @type "^[A-Z]"))

((attribute
  attribute: (identifier) @constant)
  (#match? @constant "^[A-Z][A-Z0-9_]*$"))

(class_definition
  body: (block
    (expression_statement
      (assignment
        left: (identifier) @property))))

((class_definition
  body: (block
    (expression_statement
      (assignment
        left: (identifier) @constant))))
  (#match? @constant "^[A-Z][A-Z0-9_]*$"))

(type
  (identifier) @type)

(generic_type
  (identifier) @type)

((type
  (identifier) @type.builtin)
  (#match? @type.builtin "^(int|str|float|bool|bytes|bytearray|complex|dict|list|set|frozenset|tuple|object|type)$"))

((generic_type
  (identifier) @type.builtin)
  (#match? @type.builtin "^(int|str|float|bool|bytes|bytearray|complex|dict|list|set|frozenset|tuple|object|type)$"))

(import_statement
  name: (dotted_name
    (identifier) @namespace))

(import_statement
  name: (aliased_import
    name: (dotted_name
      (identifier) @namespace)))

(import_statement
  name: (aliased_import
    alias: (identifier) @namespace))

(import_from_statement
  module_name: (dotted_name
    (identifier) @namespace))

(call
  function: (identifier) @function)

(call
  function: (attribute
    attribute: (identifier) @function.method))

((call
  function: (identifier) @type)
  (#match? @type "^[A-Z]"))

((call
  function: (attribute
    attribute: (identifier) @type))
  (#match? @type "^[A-Z]"))

((call
  function: (identifier) @function.builtin)
  (#match? @function.builtin
    "^(abs|aiter|all|anext|any|ascii|bin|bool|breakpoint|bytearray|bytes|callable|chr|classmethod|compile|complex|delattr|dict|dir|divmod|enumerate|eval|exec|filter|float|format|frozenset|getattr|globals|hasattr|hash|help|hex|id|input|int|isinstance|issubclass|iter|len|list|locals|map|max|memoryview|min|next|object|oct|open|ord|pow|print|property|range|repr|reversed|round|set|setattr|slice|sorted|staticmethod|str|sum|super|tuple|type|vars|zip|__import__)$"))

((call
  function: (identifier) @type.builtin)
  (#match? @type.builtin "^(int|str|float|bool|bytes|bytearray|complex|dict|list|set|frozenset|tuple|object|type)$"))

(function_definition
  name: (identifier) @function)

(class_definition
  body: (block
    (function_definition
      name: (identifier) @function.method)))

(class_definition
  body: (block
    (decorated_definition
      definition: (function_definition
        name: (identifier) @function.method))))

((decorated_definition
  (decorator
    (identifier) @_decorator)
  definition: (function_definition
    name: (identifier) @property))
  (#eq? @_decorator "property"))

(class_definition
  name: (identifier) @type)

(decorator
  "@" @attribute)

(decorator
  (identifier) @attribute)

(decorator
  (attribute
    attribute: (identifier) @attribute))

(decorator
  (call
    function: (identifier) @attribute))

(decorator
  (call
    function: (attribute
      attribute: (identifier) @attribute)))

(none) @constant.builtin

[
  (true)
  (false)
] @boolean

[
  (integer)
  (float)
] @number

(comment) @comment

(string) @string

(escape_sequence) @escape

[
  "-"
  "-="
  "!="
  "*"
  "**"
  "**="
  "*="
  "/"
  "//"
  "//="
  "/="
  "&"
  "&="
  "%"
  "%="
  "^"
  "^="
  "+"
  "->"
  "+="
  "<"
  "<<"
  "<<="
  "<="
  "<>"
  "="
  ":="
  "=="
  ">"
  ">="
  ">>"
  ">>="
  "|"
  "|="
  "~"
  "@="
] @operator

[
  "and"
  "as"
  "assert"
  "async"
  "await"
  "break"
  "case"
  "class"
  "continue"
  "def"
  "del"
  "elif"
  "else"
  "except"
  "exec"
  "finally"
  "for"
  "from"
  "global"
  "if"
  "import"
  "in"
  "is"
  "is not"
  "lambda"
  "match"
  "nonlocal"
  "not"
  "not in"
  "or"
  "pass"
  "print"
  "raise"
  "return"
  "try"
  "type"
  "while"
  "with"
  "yield"
] @keyword
