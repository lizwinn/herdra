//! Resolves one key press against the active menu stack.
//!
//! The top level is where keys go to the pane. A top-level chord opens a
//! menu. Inside menus, the top menu's bindings apply first, then the menus
//! under it while each one says `fallthrough`, then top-level chords (so the
//! prefix and prefix-free chords work everywhere), then the common `esc` and
//! `backspace` bindings.
//!
//! Opening a submenu stacks it on the menu it was opened from, so
//! `backspace` goes back one menu. When a leaf closes its menu, the one-shot
//! menus that were only passed through on the way close with it. Opening a
//! `mode` menu starts from under those passed-through menus, so leaving the
//! mode does not reveal them again.

use super::chord::{chord_match, generated_character_key, Chord, MatchPass};
use super::compile::{CompiledBinding, CompiledKeymap, CompiledTarget, MenuId};
use super::parse::{ExitPolicy, Unmatched};
use crate::input::TerminalKey;

/// Room for a full-depth path opened over a mode and the menus in between.
pub(crate) const MAX_STACK: usize = super::parse::MAX_MENU_DEPTH + 4;

/// Open menus, bottom first. Never empty; the top level is not a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct MenuStack {
    len: u8,
    ids: [MenuId; MAX_STACK],
}

impl MenuStack {
    pub(crate) fn single(id: MenuId) -> Self {
        let mut ids = [MenuId::TOP; MAX_STACK];
        ids[0] = id;
        Self { len: 1, ids }
    }

    pub(crate) fn top(&self) -> MenuId {
        self.ids[usize::from(self.len) - 1]
    }

    pub(crate) fn frames(&self) -> &[MenuId] {
        &self.ids[..usize::from(self.len)]
    }

    /// Push a menu, or replace the top when the stack is full.
    pub(crate) fn pushed(mut self, id: MenuId) -> Self {
        if usize::from(self.len) < MAX_STACK {
            self.ids[usize::from(self.len)] = id;
            self.len += 1;
        } else {
            self.ids[MAX_STACK - 1] = id;
        }
        self
    }

    /// Remove the top frame; `None` when that leaves nothing open.
    pub(crate) fn popped(mut self) -> Option<Self> {
        if self.len <= 1 {
            return None;
        }
        self.len -= 1;
        Some(self)
    }

    /// Keep the bottom `len` frames; `None` when that keeps nothing.
    pub(crate) fn truncated(mut self, len: usize) -> Option<Self> {
        if len == 0 {
            return None;
        }
        self.len = self.len.min(u8::try_from(len).unwrap_or(u8::MAX));
        Some(self)
    }

    /// Keep frames up to and including the highest `mode` frame.
    pub(crate) fn unwound_to_mode(self, keymap: &CompiledKeymap) -> Option<Self> {
        let mut stack = Some(self);
        while let Some(current) = stack {
            if keymap.menu(current.top()).anchor {
                return Some(current);
            }
            stack = current.popped();
        }
        None
    }

    /// Close the top frame and everything down to the next `mode` frame.
    pub(crate) fn cancelled(self, keymap: &CompiledKeymap) -> Option<Self> {
        self.popped()
            .and_then(|stack| stack.unwound_to_mode(keymap))
    }

    /// Close the top frame after one of its leaves ran, and the one-shot
    /// menus that were only passed through to reach it.
    pub(crate) fn left(self, keymap: &CompiledKeymap) -> Option<Self> {
        self.popped()
            .and_then(|stack| stack.without_passed_through(keymap))
    }

    /// The frames that pressing the keys to `menu` opens: its ancestors
    /// below the top level, starting at the nearest `mode` menu.
    pub(crate) fn opened_at(keymap: &CompiledKeymap, menu: MenuId) -> Self {
        let mut chain = Vec::new();
        let mut current = Some(menu);
        while let Some(id) = current.filter(|id| *id != MenuId::TOP) {
            chain.push(id);
            if keymap.menu(id).anchor || chain.len() == MAX_STACK {
                break;
            }
            current = keymap.menu(id).parent;
        }
        let mut frames = chain.into_iter().rev();
        let mut stack = Self::single(frames.next().unwrap_or(menu));
        for id in frames {
            stack = stack.pushed(id);
        }
        stack
    }

    /// Drop one-shot frames from the top until a sticky or `mode` frame.
    pub(crate) fn without_passed_through(self, keymap: &CompiledKeymap) -> Option<Self> {
        let mut stack = self;
        loop {
            let menu = keymap.menu(stack.top());
            if menu.sticky || menu.anchor {
                return Some(stack);
            }
            stack = stack.popped()?;
        }
    }

    /// Open `child` from the top frame.
    fn entered(self, keymap: &CompiledKeymap, child: MenuId) -> Self {
        if !keymap.menu(child).anchor {
            return self.pushed(child);
        }
        match self.without_passed_through(keymap) {
            Some(base) => base.pushed(child),
            None => Self::single(child),
        }
    }
}

/// What a key press does.
#[derive(Debug)]
pub(crate) enum Step<'a> {
    /// Send the key to the pane; nothing is open.
    Forward,
    /// Consume the key and change nothing.
    Ignore,
    /// Replace the open menus, then run the effect.
    Apply {
        next: Option<MenuStack>,
        effect: Effect<'a>,
    },
}

#[derive(Debug)]
pub(crate) enum Effect<'a> {
    None,
    /// Run a leaf. `index` is the digit pressed on a `1..9` chord.
    Run {
        binding: &'a CompiledBinding,
        index: Option<usize>,
    },
    /// Send the chord that opened `menu` to the pane.
    Literal {
        menu: MenuId,
    },
    /// Send the pressed key to the pane.
    ForwardKey,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Source {
    /// Found in the frame at this depth (1 is the bottom frame).
    Menu(usize),
    TopLevel,
    Common,
}

fn find<'a>(
    bindings: &'a [CompiledBinding],
    key: &TerminalKey,
) -> Option<(&'a CompiledBinding, Option<usize>)> {
    for pass in [MatchPass::Exact, MatchPass::LooseDigits] {
        for binding in bindings {
            if pass == MatchPass::Exact && matches!(binding.chord, Chord::Digits(_)) {
                continue;
            }
            if let Some(index) = chord_match(binding.chord, key, pass) {
                return Some((binding, index));
            }
        }
        if pass == MatchPass::Exact {
            for binding in bindings {
                if !matches!(binding.chord, Chord::Digits(_)) {
                    continue;
                }
                if let Some(index) = chord_match(binding.chord, key, MatchPass::Exact) {
                    return Some((binding, index));
                }
            }
        }
    }
    None
}

fn find_in_stack<'a>(
    keymap: &'a CompiledKeymap,
    stack: MenuStack,
    key: &TerminalKey,
) -> Option<(&'a CompiledBinding, Option<usize>, Source)> {
    let frames = stack.frames();
    for depth in (1..=frames.len()).rev() {
        let menu = keymap.menu(frames[depth - 1]);
        if let Some((binding, index)) = find(&menu.bindings, key) {
            return Some((binding, index, Source::Menu(depth)));
        }
        if !menu.fallthrough {
            break;
        }
    }
    if let Some((binding, index)) = find(&keymap.top().bindings, key) {
        return Some((binding, index, Source::TopLevel));
    }
    find(&keymap.common, key).map(|(binding, index)| (binding, index, Source::Common))
}

/// Resolve a key press given the open menus (`None` when typing in a pane).
pub(crate) fn resolve<'a>(
    keymap: &'a CompiledKeymap,
    stack: Option<MenuStack>,
    key: &TerminalKey,
) -> Step<'a> {
    let Some(stack) = stack else {
        return match find(&keymap.top().bindings, key) {
            None => Step::Forward,
            Some((binding, index)) => at_top_level(binding, index),
        };
    };
    let found = find_in_stack(keymap, stack, key)
        .or_else(|| {
            generated_character_key(key)
                .and_then(|generated| find_in_stack(keymap, stack, &generated))
        })
        .or_else(|| escape_alias(key).and_then(|escape| find_in_stack(keymap, stack, &escape)));
    let Some((binding, index, source)) = found else {
        return match keymap.menu(stack.top()).unmatched {
            Unmatched::Ignore => Step::Ignore,
            Unmatched::Cancel => Step::Apply {
                next: stack.cancelled(keymap),
                effect: Effect::None,
            },
            Unmatched::Forward => Step::Apply {
                next: stack.cancelled(keymap),
                effect: Effect::ForwardKey,
            },
        };
    };
    match source {
        Source::TopLevel => from_top_level_inside(keymap, stack, binding, index),
        Source::Common => apply(keymap, stack, binding, index),
        // A key found under a `fallthrough` menu acts from its own menu.
        Source::Menu(depth) => apply(
            keymap,
            stack.truncated(depth).unwrap_or(stack),
            binding,
            index,
        ),
    }
}

/// `ctrl+[` sends the same byte as `esc` in legacy terminals; inside menus,
/// treat it as `esc` everywhere.
fn escape_alias(key: &TerminalKey) -> Option<TerminalKey> {
    (key.code == crossterm::event::KeyCode::Char('[')
        && key.modifiers == crossterm::event::KeyModifiers::CONTROL)
        .then(|| {
            TerminalKey::new(
                crossterm::event::KeyCode::Esc,
                crossterm::event::KeyModifiers::empty(),
            )
        })
}

/// Run a binding found in the top frame of `stack`.
fn apply<'a>(
    keymap: &'a CompiledKeymap,
    stack: MenuStack,
    binding: &'a CompiledBinding,
    index: Option<usize>,
) -> Step<'a> {
    let top = stack.top();
    let menu = keymap.menu(top);
    match binding.target {
        CompiledTarget::Enter(child) => Step::Apply {
            next: Some(if child == top {
                stack
            } else {
                stack.entered(keymap, child)
            }),
            effect: Effect::None,
        },
        CompiledTarget::Back => Step::Apply {
            next: stack.popped(),
            effect: Effect::None,
        },
        CompiledTarget::Cancel => Step::Apply {
            next: stack.cancelled(keymap),
            effect: Effect::None,
        },
        CompiledTarget::Literal => Step::Apply {
            next: stack.cancelled(keymap),
            effect: Effect::Literal { menu: top },
        },
        CompiledTarget::Action(_) | CompiledTarget::Command(_) => {
            let stays = binding.exits_itself
                || match binding.exit {
                    Some(ExitPolicy::Stay) => true,
                    Some(ExitPolicy::Exit) => false,
                    None => menu.sticky,
                };
            Step::Apply {
                next: if stays {
                    Some(stack)
                } else {
                    stack.left(keymap)
                },
                effect: Effect::Run { binding, index },
            }
        }
    }
}

fn at_top_level(binding: &CompiledBinding, index: Option<usize>) -> Step<'_> {
    match binding.target {
        CompiledTarget::Enter(menu) => Step::Apply {
            next: Some(MenuStack::single(menu)),
            effect: Effect::None,
        },
        CompiledTarget::Action(_) | CompiledTarget::Command(_) => Step::Apply {
            next: None,
            effect: Effect::Run { binding, index },
        },
        CompiledTarget::Back | CompiledTarget::Cancel => Step::Ignore,
        CompiledTarget::Literal => Step::Forward,
    }
}

/// A top-level chord pressed while menus are open: menus reopen from the
/// top, keeping `mode` frames; leaves run without closing anything.
fn from_top_level_inside<'a>(
    keymap: &'a CompiledKeymap,
    stack: MenuStack,
    binding: &'a CompiledBinding,
    index: Option<usize>,
) -> Step<'a> {
    match binding.target {
        CompiledTarget::Enter(menu) => {
            let next = match stack.unwound_to_mode(keymap) {
                Some(base) if base.top() == menu => base,
                Some(base) => base.pushed(menu),
                None => MenuStack::single(menu),
            };
            Step::Apply {
                next: Some(next),
                effect: Effect::None,
            }
        }
        CompiledTarget::Action(_) | CompiledTarget::Command(_) => Step::Apply {
            next: Some(stack),
            effect: Effect::Run { binding, index },
        },
        CompiledTarget::Back => Step::Apply {
            next: stack.popped(),
            effect: Effect::None,
        },
        CompiledTarget::Cancel => Step::Apply {
            next: stack.cancelled(keymap),
            effect: Effect::None,
        },
        CompiledTarget::Literal => Step::Apply {
            next: stack.cancelled(keymap),
            effect: Effect::ForwardKey,
        },
    }
}
