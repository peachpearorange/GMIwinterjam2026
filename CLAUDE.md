# Jam Rules (GMI Winter Jam 2026)
Theme: « Snowed In ». Submissions due 16:30 on November 25th 2026.

TOOLS & RESOURCES

✅ All engines are allowed
✅ You can use third-party assets (see rules about Generative AI)
✅ You may use your own pre-existing assets

GENERATIVE AI POLICY (HONOR SYSTEM)

No AI Creative Assets: To keep the focus on human creativity, please do not use generative AI (such as Midjourney, Stable Diffusion, ChatGPT, Suno, ElevenLabs, etc.) to create art, 3D models, UI, music, sound effects, voice acting, or writing. All creative assets should be made by your team or taken from human-made asset packs (with proper credit).

AI Allowed for Code: You are welcome to use AI tools (such as GitHub Copilot, ChatGPT, or IDE autocomplete) for programming, debugging, and writing code.

Honor System: We cannot verify every file, so this rule relies on the honor system. Please respect the spirit of the jam and keep your creative assets human-made!

So: write code only. Never author art, models, materials, palettes, music, sound, or in-game text (dialogue, story, names, titles); leave placeholders for me. Editor tooling UI is code, not game content.

# Design Principles
Inferred from past corrections. Use them to decide details without asking.

## Tools
- "Simple" means pixel-editor style: pick a tool, then click or drag on whatever is under the mouse. No select-then-act, no hidden order of steps, no mode switching. If using a feature means following a recipe, the design is wrong.
- One tool per action, working on every kind of part (vertex, edge, face, whole shape). Shift-click and lasso handle multi-select. When two tools are nearly the same, propose merging them.
- If the answer to "how do I X?" takes 4+ steps, give the workaround briefly and offer a direct tool.
- Existing editors (the pixel editor first) set the pattern. New editors and tools should look and behave like their siblings.
- Give tools descriptive names.

## Don't add limits nobody asked for
- Movement defaults to free and direct, with snapping as an option. Don't lock movement to axes or normals unless asked. "Toward the camera" means straight along the view ray.
- Don't keep restrictions that exist only out of caution (e.g. no spaces in names). Before improving a limit's error message, ask whether the limit needs to exist.

## Quality bar
- Judge results by how they look and feel in use, not only by whether tests pass. For geometry operations, think about the next likely action (e.g. dragging the new corner) and make that look clean. Update neighbouring faces' topology as needed.
- Fix the cause, not the symptom. If a UI bug comes back after a patch, change the structure (e.g. give it its own tab).

## UI layout
- Use the available space: no big blank areas, use the space beside widgets, and make lists tall.
- Panels must not grow over time.
- Prefer a dedicated tab, like the existing editors, over popup windows.
- Cut clutter hint text.
- Single click to switch items in lists, as long as nothing would be lost.

## Interpreting requests
- A short instruction ("make it simple", "make it good") means do it properly, to a high bar, without offering a menu of options.
- Ask only when these principles don't settle the question.
- A short follow-up usually refers to the most recent change.

## Workflow
- Don't do timed runs (`timeout N cargo run`). Run `cargo check` / `cargo test`, then hand off for playtesting.
