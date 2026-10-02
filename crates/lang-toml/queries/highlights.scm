(comment) @comment

(table
  [
    (bare_key)
    (quoted_key)
    (dotted_key)
  ] @namespace)

(table_array_element
  [
    (bare_key)
    (quoted_key)
    (dotted_key)
  ] @namespace)

(pair
  [
    (bare_key)
    (quoted_key)
    (dotted_key)
  ] @property.declaration)

(string) @string

(escape_sequence) @string.escape

[
  (integer)
  (float)
] @number

(boolean) @boolean

[
  (offset_date_time)
  (local_date_time)
  (local_date)
  (local_time)
] @string.special

"=" @operator
