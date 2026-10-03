//! Bundles the `macros` Lua module, which runs named sequences of commands
//! in a pane, into wezterm, so that `require("macros")` works out of the box.
use config::lua::mlua::{Lua, MultiValue, Table, Value};

const SOURCE: &str = include_str!("lua/macros.lua");

pub fn register(lua: &Lua) -> anyhow::Result<()> {
    let package: Table = lua.globals().get("package")?;
    let preload: Table = package.get("preload")?;
    preload.set(
        "macros",
        lua.create_function(|lua, _: MultiValue| {
            lua.load(SOURCE)
                .set_name("@macros.lua")
                .call::<_, Value>(())
        })?,
    )?;
    Ok(())
}

#[cfg(test)]
mod test {
    use super::*;

    /// Creates a Lua context with the macros module and a stub of the
    /// parts of the wezterm module that it uses. `fake_pane(script)`
    /// creates a pane whose shell prints the lines in `script[command]`
    /// a few polls after `command` is typed, followed by `script.prompt`.
    fn lua_with_fake_pane() -> Lua {
        let lua = Lua::new();
        register(&lua).unwrap();
        lua.load(
            r#"
            local polls = 0
            local pending = {}
            on_poll = nil
            package.loaded.wezterm = {
                GLOBAL = {},
                log_warn = function() end,
                action_callback = function(f) return f end,
                emit = function() end,
                sleep_ms = function(ms)
                    polls = polls + 1
                    if on_poll then on_poll(polls) end
                    -- deliver the delayed output
                    local still = {}
                    for _, p in ipairs(pending) do
                        if polls >= p.at then p.fn() else table.insert(still, p) end
                    end
                    pending = still
                end,
                time = {
                    call_after = function(_, fn) fn() end,
                },
            }

            function fake_pane(script)
                local pane = { lines = { script.prompt }, sent = {}, input = "" }
                function pane:pane_id() return 1 end
                function pane:window() return nil end
                function pane:get_cursor_position()
                    return { x = #self.lines[#self.lines], y = #self.lines - 1 }
                end
                function pane:get_dimensions()
                    return {
                        cols = 80,
                        viewport_rows = 24,
                        scrollback_top = 0,
                        physical_top = math.max(0, #self.lines - 24),
                    }
                end
                function pane:get_text_from_region(_, first, _, last)
                    local out = {}
                    for row = first, last do
                        if self.lines[row + 1] then table.insert(out, self.lines[row + 1]) end
                    end
                    return table.concat(out, "\n")
                end
                function pane:print(line)
                    table.insert(self.lines, line)
                end
                function pane:send_text(text)
                    local typed = text:gsub("\r$", "")
                    table.insert(self.sent, { text = text, at_poll = polls })
                    -- echo on the current line, as a shell does
                    self.lines[#self.lines] = self.lines[#self.lines] .. typed
                    self.input = self.input .. typed
                    if text:sub(-1) ~= "\r" then return end
                    local reply = script[self.input] or {}
                    self.input = ""
                    table.insert(pending, {
                        at = polls + (reply.delay or 3),
                        fn = function()
                            for _, l in ipairs(reply) do self:print(l) end
                            self:print(reply.prompt or script.prompt)
                        end,
                    })
                end
                return pane
            end

            macros = require("macros")
            macros.poll_interval_ms = 100
            "#,
        )
        .exec()
        .unwrap();
        lua
    }

    #[test]
    fn compiles() {
        Lua::new()
            .load(SOURCE)
            .set_name("macros.lua")
            .into_function()
            .unwrap();
    }

    #[test]
    fn waits_for_remote_prompt_before_continuing() {
        let lua = lua_with_fake_pane();
        lua.load(
            r#"
            local pane = fake_pane({
                prompt = "me@laptop:~$ ",
                ["ssh web01"] = { delay = 5, "Welcome to web01", prompt = "deploy@web01:~$ " },
                ["uptime"] = { delay = 1, " 10:00:00 up 3 days", prompt = "deploy@web01:~$ " },
            })
            macros.define("web01", {
                description = "uptime on web01",
                steps = {
                    "ssh web01",
                    { wait_for = macros.PROMPT, timeout = 5 },
                    "uptime",
                    { wait_for = "up %d+ days", timeout = 5 },
                },
            })
            local result
            assert(macros.run(pane, "web01", { on_done = function(ok, err) result = { ok, err } end }))
            assert(result[1] == true, tostring(result[2]))
            assert(#pane.sent == 2)
            assert(pane.sent[1].text == "ssh web01\r")
            assert(pane.sent[2].text == "uptime\r")
            -- uptime was only typed once the remote prompt had been printed,
            -- not as soon as the local prompt matched
            assert(pane.sent[2].at_poll >= 5, "sent too early: " .. pane.sent[2].at_poll)
            assert(not macros.is_running(pane))
            "#,
        )
        .exec()
        .unwrap();
    }

    #[test]
    fn times_out() {
        let lua = lua_with_fake_pane();
        lua.load(
            r#"
            local pane = fake_pane({ prompt = "$ " })
            macros.define("never", { "true", { wait_for = "will not appear", timeout = 0.5 }, "echo unreachable" })
            local result
            macros.run(pane, "never", { on_done = function(ok, err) result = { ok, err } end })
            assert(result[1] == false)
            assert(result[2]:find("timed out"), result[2])
            assert(#pane.sent == 1)
            assert(not macros.is_running(pane))
            "#,
        )
        .exec()
        .unwrap();
    }

    #[test]
    fn can_be_cancelled_while_waiting() {
        let lua = lua_with_fake_pane();
        lua.load(
            r#"
            local pane = fake_pane({ prompt = "$ " })
            macros.define("slow", { { sleep = 10 }, "echo late" })
            on_poll = function(n)
                if n == 3 then
                    assert(macros.running_macro(pane) == "slow")
                    assert(macros.cancel(pane))
                end
            end
            local result
            macros.run(pane, "slow", { on_done = function(ok, err) result = { ok, err } end })
            assert(result[1] == false and result[2] == "cancelled")
            assert(#pane.sent == 0)
            assert(not macros.cancel(pane))
            "#,
        )
        .exec()
        .unwrap();
    }

    #[test]
    fn plain_text_and_raw_sends() {
        let lua = lua_with_fake_pane();
        lua.load(
            r#"
            local pane = fake_pane({ prompt = "$ ", ["ls"] = { "a.txt [x]" } })
            macros.define("plain", {
                { send = "l", enter = false },
                { send = "s" },
                { wait_for = "[x]", plain = true, timeout = 2 },
            })
            local result
            macros.run(pane, "plain", { on_done = function(ok, err) result = { ok, err } end })
            assert(result[1] == true, tostring(result[2]))
            assert(pane.sent[1].text == "l" and pane.sent[2].text == "s\r")
            "#,
        )
        .exec()
        .unwrap();
    }

    #[test]
    fn rejects_invalid_definitions() {
        let lua = lua_with_fake_pane();
        lua.load(
            r#"
            local function fails(spec, expected)
                local ok, err = pcall(macros.define, "bad", spec)
                assert(not ok, "accepted " .. expected)
                assert(tostring(err):find(expected, 1, true), tostring(err))
            end
            fails({}, "non-empty list")
            fails({ steps = { 42 } }, "expected a string or a table")
            fails({ { send = "x", sleep = 1 } }, "exactly one of")
            fails({ { wait_for = "x", timeout = 0 } }, "positive number")
            fails({ { wait_for = "%" } }, "invalid pattern")
            assert(macros.get("bad") == nil)

            macros.define("b", { "x" })
            macros.define("a", { description = "first", key = "m", mods = "ALT", steps = { "y" } })
            local names = macros.list()
            assert(#names == 2 and names[1] == "a" and names[2] == "b")
            local entries = macros.command_palette_entries()
            assert(#entries == 2 and entries[1].brief == "Macro: a" and entries[1].doc == "first")

            -- unknown macros and concurrent runs are refused
            local pane = fake_pane({ prompt = "$ " })
            assert(not macros.run(pane, "missing"))
            "#,
        )
        .exec()
        .unwrap_or_else(|err| panic!("{err:#}"));
    }
}
