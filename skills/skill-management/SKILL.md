# Skill Management

Create, update, delete, and list agent skills using the `skill_manage` tool.

## Key Rule

**ALWAYS use `skill_manage` to create or modify skills. NEVER write SKILL.md or SKILL.toml files directly via shell or file tools.** Direct file writes bypass hot-reload — the new skill won't appear until restart.

## Why

`skill_manage` writes the skill files AND triggers hot-reload so the skill is available on the next turn without restart.

## skill_manage Usage

```
skill_manage action="list"                                    # list all loaded skills
skill_manage action="create" name="X" content="# My Skill\nInstructions here"  # create from markdown
skill_manage action="create" name="X" description="Y"        # create from structured TOML
skill_manage action="read" name="X"                           # read skill content
skill_manage action="update" name="X" content="# Updated\n..." # update skill
skill_manage action="delete" name="X"                         # remove skill
```

## Examples

### Create a markdown skill

```
skill_manage action="create" name="my-helper" content="# My Helper\nWhen asked to do X, follow these steps:\n1. Step one\n2. Step two"
```

### Create a structured skill with tools

```
skill_manage action="create" name="deploy-checker" description="Validate deployments" tools=[{"name":"check","description":"Run checks","kind":"shell","command":"./check.sh"}]
```

## Rules

- Skill names: alphanumeric, hyphens, underscores only (1-64 chars).
- After create/update/delete, the skill is hot-reloaded automatically.
- `action="list"` reads current in-memory state (does not trigger reload).
