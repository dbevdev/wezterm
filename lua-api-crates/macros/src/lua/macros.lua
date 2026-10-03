-- Command macros: named sequences of steps that type commands into a pane
-- and optionally wait for output before continuing.
--
-- Because a macro types into the pane, the commands run wherever the pane
-- is connected to: the local shell, or a remote host after `ssh`.
--
--   local macros = require("macros")
--   macros.define("web01-logs", {
--     description = "Tail the app log on web01",
--     steps = {
--       "ssh web01",
--       { wait_for = macros.PROMPT, timeout = 30 },
--       "cd /srv/app && tail -f log/app.log",
--     },
--   })

local wezterm = require("wezterm")

local pub = {}

--- Lua pattern matching the end of common shell prompts ($, #, > or %),
--- for use with `wait_for`. Trailing whitespace is ignored when matching.
pub.PROMPT = "[%$#>%%]$"

--- Interval between checks of the pane output while waiting, in milliseconds
pub.poll_interval_ms = 100

--- Default timeout of `wait_for` steps, in seconds
pub.default_timeout = 30

--- At most this many lines before the cursor are searched by `wait_for`
pub.max_search_lines = 500

---@type table<string, table>
local registry = {}

-- The state of running macros is kept in wezterm.GLOBAL, so that a macro
-- started before a configuration reload can still be cancelled after it.
local function running_table()
	if wezterm.GLOBAL.macros_running == nil then
		wezterm.GLOBAL.macros_running = {}
	end
	return wezterm.GLOBAL.macros_running
end

local function pane_key(pane)
	return tostring(pane:pane_id())
end

---@param name string
---@param step any
---@param index integer
---@return table
local function normalize_step(name, step, index)
	local function fail(msg)
		error(string.format("macro %q, step %d: %s", name, index, msg), 3)
	end
	if type(step) == "string" then
		return { send = step, enter = true }
	end
	if type(step) ~= "table" then
		fail("expected a string or a table, got " .. type(step))
	end
	local kinds = 0
	for _, k in ipairs({ "send", "wait_for", "sleep" }) do
		if step[k] ~= nil then
			kinds = kinds + 1
		end
	end
	if kinds ~= 1 then
		fail("must have exactly one of `send`, `wait_for` or `sleep`")
	end
	if step.send ~= nil then
		if type(step.send) ~= "string" then
			fail("`send` must be a string")
		end
		return { send = step.send, enter = step.enter ~= false }
	elseif step.wait_for ~= nil then
		if type(step.wait_for) ~= "string" or step.wait_for == "" then
			fail("`wait_for` must be a non-empty string")
		end
		local timeout = step.timeout or pub.default_timeout
		if type(timeout) ~= "number" or timeout <= 0 then
			fail("`timeout` must be a positive number of seconds")
		end
		if not step.plain then
			-- Report invalid patterns now rather than when the macro runs
			local ok, err = pcall(string.find, "", step.wait_for)
			if not ok then
				fail("invalid pattern " .. step.wait_for .. ": " .. tostring(err))
			end
		end
		return { wait_for = step.wait_for, timeout = timeout, plain = step.plain == true }
	else
		if type(step.sleep) ~= "number" or step.sleep < 0 then
			fail("`sleep` must be a non-negative number of seconds")
		end
		return { sleep = step.sleep }
	end
end

--- Defines (or replaces) a macro.
--- `spec` is either a list of steps or a table with the fields:
---   steps: list of steps (required)
---   description: shown in the macro selector and the command palette
---   confirm: if true, ask for confirmation before running it
---   key, mods: optional key binding, applied by `macros.apply_to_config`
--- A step is one of:
---   "text"                          type `text` and press Enter
---   { send = "text", enter = false } type `text`, Enter unless enter = false
---   { wait_for = "pattern", timeout = 30, plain = false }
---                                   wait until a line of output that appeared
---                                   after the previous step matches the Lua
---                                   pattern (or contains the text if plain)
---   { sleep = 1.5 }                  wait for the given number of seconds
---@param name string
---@param spec table
function pub.define(name, spec)
	if type(name) ~= "string" or name == "" then
		error("macro name must be a non-empty string", 2)
	end
	if type(spec) ~= "table" then
		error(string.format("macro %q: expected a table", name), 2)
	end
	local steps = spec.steps
	if steps == nil and spec[1] ~= nil then
		steps = spec
	end
	if type(steps) ~= "table" or #steps == 0 then
		error(string.format("macro %q: `steps` must be a non-empty list", name), 2)
	end
	local normalized = {}
	for i, step in ipairs(steps) do
		normalized[i] = normalize_step(name, step, i)
	end
	registry[name] = {
		name = name,
		description = spec.description or "",
		confirm = spec.confirm == true,
		key = spec.key,
		mods = spec.mods,
		steps = normalized,
	}
	return registry[name]
end

--- Returns the definition of a macro, or nil
---@param name string
function pub.get(name)
	return registry[name]
end

--- Returns the sorted list of the names of the defined macros
---@return string[]
function pub.list()
	local names = {}
	for name in pairs(registry) do
		table.insert(names, name)
	end
	table.sort(names)
	return names
end

---@param pane any
---@return boolean
function pub.is_running(pane)
	return running_table()[pane_key(pane)] ~= nil
end

--- Returns the name of the macro running in the pane, or nil
function pub.running_macro(pane)
	local entry = running_table()[pane_key(pane)]
	return entry and entry.name
end

--- Stops the macro running in the pane, if any.
--- It stops before its next step, or while it is waiting.
---@return boolean true if a macro was running
function pub.cancel(pane)
	local running = running_table()
	local key = pane_key(pane)
	if running[key] == nil then
		return false
	end
	running[key] = nil
	return true
end

local function gui_window_for(pane)
	local ok, win = pcall(function()
		local mux_window = pane:window()
		return mux_window and mux_window:gui_window()
	end)
	if ok then
		return win
	end
end

local function notify(pane, message)
	wezterm.log_warn("macros: " .. message)
	local win = gui_window_for(pane)
	if win then
		pcall(function()
			win:toast_notification("wezterm macro", message, nil, 5000)
		end)
	end
end

--- Checks whether the output of the pane matches `pattern`, considering
--- only lines below `state.mark` (or starting at it if nothing was sent
--- yet), so that text that was already on the screen, such as the prompt
--- that a command was typed at, doesn't match.
local function output_matches(pane, state, pattern, plain)
	local cursor = pane:get_cursor_position()
	local dims = pane:get_dimensions()
	local first = state.mark
	if state.after_mark then
		first = first + 1
	end
	first = math.max(first, dims.scrollback_top, cursor.y - pub.max_search_lines)
	local last = math.max(cursor.y, dims.physical_top + dims.viewport_rows - 1)
	if first > last then
		return false
	end
	local text = pane:get_text_from_region(0, first, dims.cols, last)
	for line in (text .. "\n"):gmatch("([^\n]*)\n") do
		line = line:gsub("%s+$", "")
		if line:find(pattern, 1, plain) then
			return true
		end
	end
	return false
end

local function set_mark(pane, state)
	state.mark = pane:get_cursor_position().y
	state.after_mark = true
end

-- Runs the steps of a macro; this is called asynchronously.
-- Returns true, or false and an error message.
local function execute(pane, macro, run_id)
	local running = running_table()
	local key = pane_key(pane)
	local function cancelled()
		local entry = running[key]
		return entry == nil or entry.id ~= run_id
	end

	local state = { mark = pane:get_cursor_position().y, after_mark = false }

	for i, step in ipairs(macro.steps) do
		if cancelled() then
			return false, "cancelled"
		end
		if step.send then
			set_mark(pane, state)
			pane:send_text(step.send .. (step.enter and "\r" or ""))
		elseif step.wait_for then
			local waited = 0
			local limit = step.timeout * 1000
			while not output_matches(pane, state, step.wait_for, step.plain) do
				if cancelled() then
					return false, "cancelled"
				end
				if waited >= limit then
					return false,
						string.format("step %d: timed out after %gs waiting for %q", i, step.timeout, step.wait_for)
				end
				wezterm.sleep_ms(pub.poll_interval_ms)
				waited = waited + pub.poll_interval_ms
			end
			-- Following waits only consider output after this point
			set_mark(pane, state)
		elseif step.sleep then
			local remaining = step.sleep * 1000
			while remaining > 0 do
				if cancelled() then
					return false, "cancelled"
				end
				local chunk = math.min(remaining, pub.poll_interval_ms)
				wezterm.sleep_ms(chunk)
				remaining = remaining - chunk
			end
		end
	end
	return true
end

--- Starts running the macro `name` in `pane`, in the background.
--- opts.on_done(ok, err) is called when it finishes.
--- Returns true, or false and an error message if it could not be started
--- (unknown macro, or another macro is already running in the pane).
---@param pane any
---@param name string
---@param opts? { on_done: fun(ok: boolean, err: string|nil)?, delay: number? }
---@return boolean
---@return string|nil
function pub.run(pane, name, opts)
	opts = opts or {}
	local macro = registry[name]
	if macro == nil then
		local err = string.format("unknown macro %q", tostring(name))
		notify(pane, err)
		return false, err
	end
	local running = running_table()
	local key = pane_key(pane)
	if running[key] ~= nil then
		local err = string.format("macro %q is already running in this pane", running[key].name)
		notify(pane, err)
		return false, err
	end

	local run_id = tostring(os.time()) .. "-" .. tostring(math.random(1, 1 << 30))
	running[key] = { id = run_id, name = name }
	wezterm.emit("macros.run.start", pane, name)

	wezterm.time.call_after(opts.delay or 0, function()
		local ok, result, err = pcall(execute, pane, macro, run_id)
		if not ok then
			err = tostring(result)
			result = false
		end
		local entry = running[key]
		if entry ~= nil and entry.id == run_id then
			running[key] = nil
		end
		if result then
			wezterm.emit("macros.run.finished", pane, name)
		else
			if err ~= "cancelled" then
				notify(pane, string.format("macro %q failed: %s", name, err))
			end
			wezterm.emit("macros.run.failed", pane, name, err)
		end
		if opts.on_done then
			opts.on_done(result, err)
		end
	end)
	return true
end

--- Asks for confirmation in `window` and then calls `on_yes`
---@param window any GuiWindow
---@param pane any
---@param title string
---@param on_yes fun()
function pub.confirm(window, pane, title, on_yes)
	window:perform_action(
		wezterm.action.InputSelector({
			title = title,
			choices = {
				{ id = "yes", label = "Yes, run it" },
				{ id = "no", label = "No" },
			},
			action = wezterm.action_callback(function(_, _, id)
				if id == "yes" then
					on_yes()
				end
			end),
		}),
		pane
	)
end

--- Runs a macro from a key binding or the command palette, asking for
--- confirmation first if the macro was defined with confirm = true
function pub.run_interactive(window, pane, name)
	local macro = registry[name]
	if macro and macro.confirm then
		pub.confirm(window, pane, string.format("Run macro %q?", name), function()
			pub.run(pane, name)
		end)
	else
		pub.run(pane, name)
	end
end

local function choice_label(macro)
	if macro.description ~= "" then
		return macro.name .. "  —  " .. macro.description
	end
	return macro.name
end

--- Shows a fuzzy selector of the defined macros and calls `callback(name)`
--- with the selected one
function pub.select(window, pane, title, callback)
	local choices = {}
	for _, name in ipairs(pub.list()) do
		table.insert(choices, { id = name, label = choice_label(registry[name]) })
	end
	if #choices == 0 then
		notify(pane, "no macros are defined")
		return
	end
	window:perform_action(
		wezterm.action.InputSelector({
			title = title,
			choices = choices,
			fuzzy = true,
			action = wezterm.action_callback(function(win, p, id)
				if id then
					callback(id, win, p)
				end
			end),
		}),
		pane
	)
end

-- Bindings of macros to panes and tabs.
-- They are kept in wezterm.GLOBAL so that they survive configuration
-- reloads, and are saved and restored by the bundled resurrect plugin
-- (only the macro name is saved; the steps always come from the config).

--- What to do with the macros bound to panes and tabs that resurrect
--- restores: "ask" (default) asks once for all of them, "always" runs them
--- without asking, "never" keeps the bindings but doesn't run them.
pub.restore_mode = "ask"

--- Seconds to wait after a restore before running (or asking about) the
--- bound macros, so that the restored shells have started
pub.restore_delay = 1.0

local function bindings(kind)
	if wezterm.GLOBAL.macros_bindings == nil then
		wezterm.GLOBAL.macros_bindings = { panes = {}, tabs = {} }
	end
	return wezterm.GLOBAL.macros_bindings[kind]
end

--- Binds the macro `name` to the pane (nil removes the binding)
function pub.bind_pane(pane, name)
	bindings("panes")[tostring(pane:pane_id())] = name
end

--- Returns the name of the macro bound to the pane, or nil
function pub.bound_to_pane(pane)
	return bindings("panes")[tostring(pane:pane_id())]
end

--- Binds the macro `name` to the tab (nil removes the binding).
--- When the tab is restored, the macro runs in its active pane.
function pub.bind_tab(tab, name)
	bindings("tabs")[tostring(tab:tab_id())] = name
end

--- Returns the name of the macro bound to the tab, or nil
function pub.bound_to_tab(tab)
	return bindings("tabs")[tostring(tab:tab_id())]
end

-- Macros queued by `run_restored`, run together after `restore_delay`
local restored_queue = nil

--- Called by resurrect for each restored pane that has a bound macro:
--- the binding is re-created for the new pane and the macro is queued
--- according to `restore_mode`. Unknown macro names are ignored.
---@param pane any the restored pane
---@param name string
---@param tab any? the restored tab, if the macro was bound to the tab
function pub.run_restored(pane, name, tab)
	if type(name) ~= "string" then
		return
	end
	if tab then
		pub.bind_tab(tab, name)
	else
		pub.bind_pane(pane, name)
	end
	if registry[name] == nil then
		wezterm.log_warn(string.format("macros: restored binding to unknown macro %q is not run", name))
		return
	end
	if pub.restore_mode == "never" then
		return
	end
	local first = restored_queue == nil
	if first then
		restored_queue = {}
	end
	table.insert(restored_queue, { pane = pane, name = name })
	if first then
		wezterm.time.call_after(pub.restore_delay, function()
			local queue = restored_queue
			restored_queue = nil
			pub.run_queue(queue)
		end)
	end
end

--- Runs the queued restored macros, asking first when restore_mode is "ask"
function pub.run_queue(queue)
	if queue == nil or #queue == 0 then
		return
	end
	local function run_all()
		for _, item in ipairs(queue) do
			pub.run(item.pane, item.name)
		end
	end
	if pub.restore_mode == "always" then
		run_all()
		return
	end
	local window = gui_window_for(queue[1].pane)
	if window == nil then
		wezterm.log_warn("macros: no window to ask whether to run the restored macros; not running them")
		return
	end
	local names = {}
	for _, item in ipairs(queue) do
		table.insert(names, item.name)
	end
	pub.confirm(
		window,
		queue[1].pane,
		string.format("Run the macros bound to the restored panes? (%s)", table.concat(names, ", ")),
		run_all
	)
end

--- Key assignments
pub.action = {}

--- Runs the named macro in the active pane
function pub.action.run(name)
	return wezterm.action_callback(function(window, pane)
		pub.run_interactive(window, pane, name)
	end)
end

--- Shows a selector of the defined macros and runs the chosen one in the
--- active pane
function pub.action.select()
	return wezterm.action_callback(function(window, pane)
		pub.select(window, pane, "Run macro", function(name, win, p)
			pub.run_interactive(win, p, name)
		end)
	end)
end

--- Stops the macro running in the active pane
function pub.action.cancel()
	return wezterm.action_callback(function(_, pane)
		if not pub.cancel(pane) then
			notify(pane, "no macro is running in this pane")
		end
	end)
end

-- Macro names are never empty, so "" means "no macro"
local NO_MACRO = ""

local function bind_with_selector(window, pane, kind)
	local title = kind == "tab" and "Bind a macro to this tab" or "Bind a macro to this pane"
	local choices = { { id = NO_MACRO, label = "(no macro: remove the binding)" } }
	for _, name in ipairs(pub.list()) do
		table.insert(choices, { id = name, label = choice_label(registry[name]) })
	end
	window:perform_action(
		wezterm.action.InputSelector({
			title = title,
			choices = choices,
			fuzzy = true,
			action = wezterm.action_callback(function(_, p, id)
				if id == nil then
					return
				end
				local name = id ~= NO_MACRO and id or nil
				if kind == "tab" then
					pub.bind_tab(p:tab(), name)
				else
					pub.bind_pane(p, name)
				end
			end),
		}),
		pane
	)
end

--- Shows a selector to bind a macro to the active pane, or to remove the
--- binding. Bound macros are saved by resurrect and run when it restores
--- the pane (see `restore_mode`).
function pub.action.bind_to_pane()
	return wezterm.action_callback(function(window, pane)
		bind_with_selector(window, pane, "pane")
	end)
end

--- Like `bind_to_pane`, for the active tab; the macro runs in the active
--- pane of the tab when it is restored
function pub.action.bind_to_tab()
	return wezterm.action_callback(function(window, pane)
		bind_with_selector(window, pane, "tab")
	end)
end

--- Returns command palette entries for the defined macros, for use in an
--- `augment-command-palette` event handler
function pub.command_palette_entries()
	local entries = {}
	for _, name in ipairs(pub.list()) do
		local macro = registry[name]
		table.insert(entries, {
			brief = "Macro: " .. name,
			doc = macro.description,
			action = pub.action.run(name),
		})
	end
	return entries
end

--- Appends the key bindings of the macros that were defined with `key`
--- to config.keys
function pub.apply_to_config(config)
	config.keys = config.keys or {}
	for _, name in ipairs(pub.list()) do
		local macro = registry[name]
		if macro.key then
			table.insert(config.keys, {
				key = macro.key,
				mods = macro.mods or "NONE",
				action = pub.action.run(name),
			})
		end
	end
end

return pub
