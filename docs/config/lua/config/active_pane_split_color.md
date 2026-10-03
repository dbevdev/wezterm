---
tags:
  - appearance
  - color
---
# `active_pane_split_color`

{{since('nightly')}}

When set, the parts of the split lines that border the active pane are
drawn in this color, so that the active pane is outlined.
The other parts of the split lines keep the color of the `split` entry of
the [color scheme](../../../config/appearance.md).

It is not set by default, which leaves all the split lines in the same
color.

```lua
config.active_pane_split_color = '#7aa2f7'
```

This works well together with wider dividers, see
[pane_divider_cols](pane_divider_cols.md) and
[pane_divider_rows](pane_divider_rows.md), and with
[pane_divider_line_width](pane_divider_line_width.md).
