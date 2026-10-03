# Function allow

{{since('nightly')}}

Allows [wezterm.plugin.require](require.md) to load the specified plugins.

This fork of wezterm doesn't load any plugin unless it is allowed: plugins
are fetched from the network and run with the same rights as wezterm
itself, and a plugin can in turn require other plugins that you never
looked at. Listing the plugins that you trust makes such additional
downloads fail instead.

The function takes a URL or a list of URLs:

* `"https://github.com/owner/repo"` allows exactly that repository; the
  comparison ignores case and a trailing `/` or `.git`
* an entry ending with `/`, such as `"https://github.com/owner/"`, allows
  every repository whose URL starts with it
* `"*"` allows any plugin, which is the behavior of upstream wezterm

`wezterm.plugin.allow` must be called before the first call to
`wezterm.plugin.require` in your configuration; afterwards the list can no
longer be changed, so that a plugin can't allow itself to load more code.
[wezterm.plugin.update_all](update_all.md) only updates the allowed plugins.

```lua
local wezterm = require 'wezterm'

wezterm.plugin.allow {
  'https://github.com/owner/repo',
}

local plugin = wezterm.plugin.require 'https://github.com/owner/repo'
```

Requiring a plugin that is not allowed raises an error that names it.
