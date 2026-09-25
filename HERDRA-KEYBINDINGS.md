# Current Herdr keybindings — v0.9.1

This is an inventory, not the proposed replacement. The local config had no
`[keys]` overrides when inspected. Evidence is tagged v0.9.1 source, not an
interactive test. This covers the main prefix map and navigation/resize/copy
modes, not every dialog's text-editing or selection shortcut.

`prefix = ctrl+b`. `|` means sequential presses; `+` means simultaneous keys.

## Prefix map

Every row is the key pressed after `prefix |`. Headings do not add a keystroke.

| Key | Action |
| --- | --- |
| `w` | Enter navigation mode |
| `r` | Enter resize mode |
| `[` | Enter copy mode |
| `shift+n` | New workspace |
| `shift+w` | Rename workspace |
| `shift+d` | Close workspace |
| `shift+g` | New Git worktree |
| `g` | Session navigator / goto picker |
| `c` | New tab |
| `shift+t` | Rename tab |
| `shift+x` | Close tab |
| `p` / `n` | Previous / next tab |
| `1..9` | Select tab |
| `v` / `-` | Split pane right / down |
| `x` | Close pane |
| `shift+p` | Rename pane |
| `z` | Toggle pane zoom |
| `e` | Edit scrollback |
| `h/j/k/l` | Focus pane left/down/up/right |
| `shift+h/j/k/l` | Swap pane left/down/up/right |
| `tab` / `shift+tab` | Focus next / previous pane |
| `b` | Toggle sidebar |
| `o` | Open notification target |
| `s` | Settings |
| `shift+r` | Reload config |
| `?` | Keybinding help |
| `q` | Detach, leaving processes running |
| `esc` | Cancel prefix mode |
| `ctrl+b` | Send literal prefix to pane |

An unmatched key ends prefix mode without forwarding that key to the pane.

## Navigation — `prefix | w`

| Key | Action | Mode behavior |
| --- | --- | --- |
| Up / Down | Move through workspace list | Stay |
| `h/j/k/l` | Focus pane left/down/up/right | Stay |
| Left / Right | Focus pane left/right | Stay |
| Enter | Open/confirm selected workspace | Exit on success |
| `1..9` | Switch workspace | Exit for a valid target |
| Tab / Shift+Tab | Focus next/previous pane | Exit |
| Esc or configured prefix | Leave navigation | Exit |
| Other bound keys | Dispatch shared prefix action/custom command | Generally exit or enter target mode |

Examples that exist today:

```text
prefix | w | shift+n     New workspace
prefix | w | c           New tab
prefix | w | n           Next tab
prefix | w | r           Enter resize mode
```

Numbers select tabs at the root prefix, but workspaces in navigation.
Previewing another endpoint (or an invalid target) blocks workspace/pane actions
until an available workspace is selected and confirmed with Enter.

Independent navigation settings are only:
`navigate_workspace_up`, `navigate_workspace_down`, and
`navigate_pane_left/down/up/right`. Reserved navigation keys take precedence;
general prefix pane-focus actions are excluded from the fallback resolver.
This is not an independently extensible navigation action table.

## Resize — `prefix | r`

- `h/j/k/l` or arrows resize left/down/up/right and remain in resize mode.
- Enter, Esc, or the resize-mode binding (`r` by default) exits.

## Copy — `prefix | [`

| Key | Action |
| --- | --- |
| `h/j/k/l` or arrows | Move cursor |
| `w/b/e` | Next word start / previous word start / next word end |
| `shift+w/b/e` | Equivalent big-word motions |
| `{` / `}` | Previous / next paragraph |
| `0` or Home | Start of line |
| `$` or End | End of line |
| `^` | First nonblank character |
| `g` / `shift+g` | Start / end of history |
| PageUp / PageDown | Page up / down |
| `ctrl+u` / `ctrl+d` | Half-page up / down |
| `ctrl+f` | Page down |
| `v` or Space | Start selection |
| `shift+v` | Start line selection |
| `/` / `?` | Forward / backward search prompt |
| `n` / `shift+n` | Repeat search / reverse direction |
| `y` or Enter | Copy and exit |
| `q` | Exit without copying |
| Esc | Clear selection/search first; otherwise exit |

The configured prefix retains its meaning outside the search prompt. With the
default prefix, `ctrl+b` enters prefix mode rather than paging up. The copy-mode
handler supports `ctrl+b` page-up when it is not captured as the prefix.
Inside a search prompt, Enter submits and Esc cancels the prompt.

## Actions without default bindings

- Open worktree / delete worktree checkout.
- Previous / next workspace.
- Numbered workspace jumps outside navigation.
- Previous / next agent and numbered agent focus.
- Move tab left / right.
- Last pane.
- Direct one-step pane resizing outside resize mode.

## Sources

- [Default configuration](https://github.com/herdrdev/herdr/blob/v0.9.1/src/config/model.rs)
- [Binding parser and validation](https://github.com/herdrdev/herdr/blob/v0.9.1/src/config/keybinds.rs)
- [Input and navigation/resize dispatch](https://github.com/herdrdev/herdr/blob/v0.9.1/src/client/shell/input.rs)
- [Workspace navigation](https://github.com/herdrdev/herdr/blob/v0.9.1/src/client/shell/workspace_navigation.rs)
- [Copy-mode dispatch](https://github.com/herdrdev/herdr/blob/v0.9.1/src/client/shell/copy_mode.rs)
- [Plugin contract](https://github.com/herdrdev/herdr/blob/v0.9.1/docs/versions/0.9.0/website/src/content/docs/plugins.mdx)
