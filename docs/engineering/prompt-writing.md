# Prompt writing

Read the product architecture and the manual for the model feature before you edit a prompt. For Effortline's local chat, start with [the local-model guide](local-model.md).

## Find the cause first

Do not change prompt text just because a reply or evaluation failed. Check the model request, tool schema and description, tool result, parser, and evaluation first. Fix code or missing data in its owning layer. Put required behavior such as allowed tools, valid IDs, limits, and permissions in typed code and schemas. Use prompt text for judgment and wording that code cannot enforce.

Add prompt text only when a repeatable failure shows that the model needs guidance. First remove or revise a rule that already covers the behavior. Keep each rule in one place. Do not add text to work around a bug, an evaluation problem, a provider setting, or an open product decision.

## Write for people

Prompts and tool context can appear in model replies. Use clear words that are safe to show to users. Name tools and fields for their meaning. Explain internal terms when they cannot be avoided. Write short, direct instructions that describe the result you want and how to recognize it. Avoid repeated rules, slogans, and examples unless an evaluation shows they improve the result.

State the reply format in the instruction. Do not rely on headings, tags, or formatting alone to control the answer. Mark user text and tool results as data, not instructions. Check the full request for conflicting rules, including examples and conditional context.

## Keep prompt text in its source file

Store prompt wording in the owning Markdown file, not in orchestration code. Keep fixed guidance separate from context added for a specific turn. Put changing user or tool data after the conversation history when possible. Do not copy code-owned facts such as tool names, enum values, or limits into prose; pass them through typed results or schemas.

## Check the change

Review the full prompt the model receives, not only the edited file. Run the affected synthetic evaluations with the production model and settings. Check nearby scenarios too, and repeat runs when model output can vary. Report the result and remaining risk in the PR. Keep prompts, model output, and private user data out of diagnostics.
