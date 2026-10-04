[
  "FROM"
  "AS"
  "RUN"
  "CMD"
  "LABEL"
  "EXPOSE"
  "ENV"
  "ADD"
  "COPY"
  "ENTRYPOINT"
  "VOLUME"
  "USER"
  "WORKDIR"
  "ARG"
  "ONBUILD"
  "STOPSIGNAL"
  "HEALTHCHECK"
  "SHELL"
  "MAINTAINER"
  "CROSS_BUILD"
  "NONE"
] @keyword

(comment) @comment

[
  (double_quoted_string)
  (single_quoted_string)
  (json_string)
  (heredoc_block)
] @string

[
  (heredoc_marker)
  (heredoc_end)
] @label

(escape_sequence) @string.escape

(expansion
  [
    "$"
    "{"
    "}"
  ] @operator)

(expansion_operator) @operator

(variable) @variable

((variable) @constant
  (#match? @constant "^[A-Z][A-Z_0-9]*$"))

(arg_pair
  name: (unquoted_string) @constant)

(env_pair
  name: (unquoted_string) @constant)

(label_pair
  key: (_) @property)

(param) @property

(mount_param
  "--" @property
  name: _ @property)

(image_spec
  name: (image_name) @type)

(image_tag) @label

(image_digest) @string

(image_alias) @namespace

(expose_port) @number
