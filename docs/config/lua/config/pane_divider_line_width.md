---
tags:
  - appearance
---
# `pane_divider_line_width`

{{since('nightly')}}

Specifies the thickness of the split line that is drawn in the middle of
the dividers between panes.
Defaults to the thickness of an underline in the current font.

The value is a dimension, such as `"2px"` or `"1.5pt"`.
`0` hides the line, leaving only the space of the dividers between the
panes; the dividers can still be dragged with the mouse.

```lua
config.pane_divider_line_width = '2px'
```

See also [active_pane_split_color](active_pane_split_color.md).
