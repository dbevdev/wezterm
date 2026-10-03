# Built-in command macros

`require("macros")` provides named macros: sequences of commands that are
typed into a pane, optionally waiting for some output before continuing.

A macro types into the pane, so its commands run wherever that pane is
connected to: the local shell, or a remote host after `ssh`, including an
`ssh` that the macro itself started.

## Example

```lua
local wezterm = require("wezterm")
local macros = require("macros")
local config = wezterm.config_builder()

macros.define("web01-logs", {
  description = "Connect to web01 and follow the application log",
  steps = {
    "ssh web01",
    -- wait for the remote shell prompt before typing the next command
    { wait_for = macros.PROMPT, timeout = 30 },
    "cd /srv/app",
    "tail -f log/app.log",
  },
})

macros.define("restart-app", {
  description = "Restart the app service on the current host",
  confirm = true, -- ask before running it
  key = "R", mods = "CTRL|SHIFT|ALT", -- applied by apply_to_config
  steps = {
    "sudo systemctl restart app",
    -- if sudo asks for the password, type it by hand: its prompt ends
    -- with ":", so this waits until the shell prompt is back
    { wait_for = macros.PROMPT, timeout = 120 },
    "systemctl status app --no-pager",
  },
})

config.keys = {
  -- pick a macro and run it in the active pane
  { key = "m", mods = "ALT", action = macros.action.select() },
  -- run a specific macro
  { key = "l", mods = "ALT", action = macros.action.run("web01-logs") },
  -- stop the macro running in the active pane
  { key = "m", mods = "ALT|SHIFT", action = macros.action.cancel() },
  -- bind a macro to the pane or tab, see "Restoring" below
  { key = "b", mods = "ALT", action = macros.action.bind_to_pane() },
  { key = "B", mods = "ALT|SHIFT", action = macros.action.bind_to_tab() },
}
macros.apply_to_config(config)

-- list the macros in the command palette (Ctrl+Shift+P)
wezterm.on("augment-command-palette", function(window, pane)
  return macros.command_palette_entries()
end)

return config
```

## Steps

| step | meaning |
| --- | --- |
| `"command"` | type `command` and press Enter |
| `{ send = "text", enter = false }` | type `text`; press Enter unless `enter = false` |
| `{ wait_for = "pattern", timeout = 30 }` | wait until a line of new output matches the Lua pattern |
| `{ wait_for = "text", plain = true }` | the same, for a plain substring |
| `{ sleep = 1.5 }` | wait for the given number of seconds |

`wait_for` only looks at output that appeared after the previous step
(trailing whitespace ignored), so the prompt at which the previous command
was typed doesn't count. `macros.PROMPT` matches lines ending with `$`, `#`,
`>` or `%`, which suits most shell prompts; use a more specific pattern,
such as `"deploy@web01"`, when that is ambiguous. The default timeout is 30
seconds (`macros.default_timeout`).

If a wait times out, the macro stops and a notification says which step
failed. Only one macro runs in a pane at a time.

## Restoring with resurrect

A macro can be bound to a pane (`macros.action.bind_to_pane()`) or to a tab
(`macros.action.bind_to_tab()`, the macro then runs in the tab's active
pane). The bundled [resurrect](../resurrect/README.md) saves the bindings
with the panes and tabs, and when it restores them, it binds the macros to
the new panes again and runs them, so that, for example, a pane that was
connected to a server reconnects to it.

What happens on restore is set by `macros.restore_mode`:

* `"ask"` (default): ask once whether to run all the macros of the restore
* `"always"`: run them without asking
* `"never"`: only restore the bindings

Only the name of a macro is saved; the steps always come from your
configuration, and saved names that are not defined are ignored, so a state
file can't make wezterm type arbitrary commands.

## API

* `macros.define(name, spec)`, `macros.get(name)`, `macros.list()`
* `macros.run(pane, name, { on_done = function(ok, err) end })`,
  `macros.cancel(pane)`, `macros.is_running(pane)`, `macros.running_macro(pane)`
* `macros.bind_pane(pane, name_or_nil)`, `macros.bound_to_pane(pane)`,
  `macros.bind_tab(tab, name_or_nil)`, `macros.bound_to_tab(tab)`
* `macros.action.run(name)`, `.select()`, `.cancel()`, `.bind_to_pane()`,
  `.bind_to_tab()`
* `macros.command_palette_entries()`, `macros.apply_to_config(config)`
* events: `macros.run.start(pane, name)`, `macros.run.finished(pane, name)`,
  `macros.run.failed(pane, name, err)`

## Security notes

* Don't put passwords in macros: they would be stored in plain text in your
  configuration. Prefer SSH keys, or wait for the password prompt and type
  the password by hand when asked, as in the example above.
* A macro types into whatever runs in the pane; if a program other than the
  expected shell has the focus, the text goes to it. `wait_for` steps make
  sure that the expected output is there before typing.
* Use `confirm = true` for macros that change things.
