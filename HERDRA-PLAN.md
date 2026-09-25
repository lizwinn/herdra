# Herdra: agreed direction

Planning notes from the initial discussion. No implementation has started.

## What we want

A plugin-extensible Hydra-style keymap/menu system that Herdr's defaults use too.

**No privileged modes.** Built-in, plugin, and user-defined modes must use the
same API, capabilities, and rules. If a default needs a capability, plugins and
user definitions must have it too.

- Reuse Herdr's existing native hint bar at the bottom.
- Define bindings and their displayed hints together.
- Support named menus and nested submenus.
- Let menus invoke existing Herdr actions and plugin actions.
- Support actions that stay in the menu and actions that exit it.
- Provide consistent enter, back, cancel, and exit behavior.
- Let users bind, extend, or override menus from defaults and plugins.
- Do not build a separate plugin popup as a substitute for native modes.

The motivating example is `prefix | w | n`: workspace -> new. Group commands
by the thing being manipulated, rather than scattering related actions across
unrelated shortcuts. The final bindings and configuration syntax are not decided.

Notation: `|` means release, then press the next key; `+` means concurrent keys.

## Start with what exists

Show the exact current map before proposing changes. Keep current behavior and
proposed behavior separate. See [HERDRA-KEYBINDINGS.md](HERDRA-KEYBINDINGS.md).

The installed version inspected was Herdr 0.9.1, with no local key overrides.
Navigation mode already falls back to shared prefix actions, but it does not
have a general independent action map or arbitrary user-defined nested modes.
The current plugin API offers actions and terminal UIs, not native keymap
registration. These conclusions came from source inspection, not a live test.

## Keep the fork maintainable

Taking new changes from `herdrdev/herdr` must remain manageable.

- Isolate the keymap/menu engine and keep connections to existing input,
  action dispatch, and hint rendering narrow.
- Reuse existing Herdr actions instead of duplicating their implementation.
- Preserve current behavior first by defining defaults through the new system.
- Keep changes focused; avoid unrelated cleanup, renames, and reformatting.
- Use focused commits and bring in upstream changes regularly.
- Do not preserve privileged built-in modes just to make merging easier.

These are design constraints and intended tactics, not a completed architecture.

## Fork and scope

Elizabeth intends to build this independently of upstream acceptance.
The fork is [lizwinn/herdra](https://github.com/lizwinn/herdra).

- `origin`: `git@github.com:lizwinn/herdra.git`
- `upstream`: `git@github.com:herdrdev/herdr.git`
- Local checkout: `~/Personal Workspace/herdr`

Herdr is its own multiplexer, not a tmux or screen wrapper. Zellij-like
windowing is a possible future interest, not part of this work now.

**Stop after saving and pushing these notes.** No implementation, further
research, or background work until Elizabeth resumes the project.
