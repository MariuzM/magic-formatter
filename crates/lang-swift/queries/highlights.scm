((simple_identifier) @type
  (#match? @type "^[A-Z][A-Za-z0-9_]*[a-z]"))

(call_expression
  (navigation_expression
    suffix: (navigation_suffix
      suffix: (simple_identifier) @function.method)))

((call_expression
  (simple_identifier) @type)
  (#match? @type "^[A-Z]"))

((call_expression
  (navigation_expression
    suffix: (navigation_suffix
      suffix: (simple_identifier) @type)))
  (#match? @type "^[A-Z]"))

(value_argument_label
  (simple_identifier) @variable.parameter)

(lambda_parameter
  name: (simple_identifier) @variable.parameter)

(type_parameter
  (type_identifier) @type.parameter)

(enum_entry
  name: (simple_identifier) @variant)

(prefix_expression
  "."
  target: (simple_identifier) @variant)

(pattern
  "."
  .
  (simple_identifier) @variant)

(class_declaration
  declaration_kind: "enum"
  name: (type_identifier) @type.enum)

(import_declaration
  (identifier
    (simple_identifier) @namespace))
