# Contributing to frun

frun is a small project with one maintainer who decides what it does and how it looks.
These rules keep the issue list about frun and nothing else. Issues can be written in
English or 中文.

## Issues

Use one of the two forms. Blank issues are turned off.

**Something is wrong.** A program is missing or should not be there, or frun
misbehaves. The form asks for your frun version, your Windows version, and the full
output of `frun --doctor`. For a missing program, run
`frun --doctor "C:\path\to\program.exe"`; the report starts by saying why that file
was left out. The report lists folder and program paths from your computer, so read
it before you paste it. Update to the latest version first.

**Suggestion.** Describe what you were trying to do before describing a solution.
Most suggestions are declined, because of what frun is:

- It opens programs. It does not search files or documents, the web, or settings,
  and it does not run commands or calculations. Paths are there to reach a folder.
- It keeps steps and words to a minimum. A key or a gesture is preferred over a
  setting, and a setting over an explanation.
- Anything it can keep current by itself, such as the index, the theme, or the
  screen it opens on, does not get a manual step.
- It stays light: one small exe, no installer, no runtime, no web view. A change
  that adds noticeably to its size or memory has to be worth it.
- It supports Windows 10 and 11.

An issue is closed without a reply, and may be locked or deleted, when it is:

- an advertisement, an invitation, a request to list or promote frun somewhere, or
  anything else that is not about frun;
- filed without the form, or with the required fields left empty or filled with
  placeholders;
- a duplicate of an open or closed issue;
- a question that the README answers.

Accounts that post advertisements are blocked.

## Pull requests

Pull requests from outside the project are not accepted, and the repository does not
allow them to be opened. What frun does and how it looks are decided in one place,
and a change that arrives without that discussion nearly always has to be redone.

If you want something changed, open an issue. If you have already written code,
link your branch in the issue; it can be read there.
