# Accessibility

GPUI builds its accessibility tree from each element's role and name, and
AccessKit hands it to the platform's screen reader. How to read the tree in a
cloud session is in [cloud.md](cloud.md), "The accessibility tree".

## Names and roles

- A visible string isn't an element's accessible name in GPUI: a plain
  string makes no node a screen reader can read. `TextButton::button_text`
  (`src/theme.rs`) sets both the text and the name; a later `aria_label`
  still replaces the name.
- The name reaches the tree only when the element has a `role`. Without one,
  GPUI makes no node and drops the label. So every button, popup row and menu
  row takes a role and a name: `theme::button_frame` has the button role but
  no name, and its caller gives one; `menu_row` is a menu item.
- A chip menu's rows sit in a container with the `Menu` role (the `Dialog`
  role for the chips whose popup isn't a list), so a screen reader can tell
  where the menu ends.
- A row that picks one of several is a `choice_row`: a `MenuItemRadio` with
  `aria_toggled`, so a screen reader says whether it is the one checked.

## States that change the action on Windows (#68)

AccessKit's Windows adapter drops the UI Automation Invoke pattern from a
node that has an expanded, toggled or selected state, or the `ComboBox` role,
and offers another pattern instead:

- **Expanded, or `ComboBox`.** A screen reader is offered Expand and Collapse
  instead of Invoke. They reach GPUI as `Action::Expand` and
  `Action::Collapse`, which GPUI drops unless the element handles them with
  `on_a11y_action`. So an element that only has `on_click` shouldn't take
  `aria_expanded` or the `ComboBox` role unless it handles both.
- **Selected** (`aria_selected`), and **toggled on a radio menu item**
  (`choice_row`): Select instead. AccessKit sends it as a click, but only to a
  row not already selected or checked.
- **Toggled** on other roles: Toggle, which is sent as a click.

On macOS and Linux AccessKit doesn't pass the expanded state on at all.

Two controls break the first rule today, and #68 (closed, not planned) keeps
them as they are: the sidebar's folder headings (`aria_expanded`) and
Settings' dropdowns (`Role::ComboBox` with `aria_expanded`).
`theme::tests::only_the_known_controls_say_they_are_open_without_handling_expand`
fails when another file adds one without an `on_a11y_action` handler.
