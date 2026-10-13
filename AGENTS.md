# Input action migrations

- Use frame timing for frame-driven responses and tick timing for simulation-driven responses.
- If an action migration suggests response timing that is neither frame nor tick (for example, applying actions inside raw input callbacks), ask the user to evaluate it before implementing that timing.
- Preserve existing behavior otherwise; raise proposed behavior changes with the user rather than introducing them implicitly.
