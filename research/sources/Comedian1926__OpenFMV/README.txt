# OpenFMV

<p align="center">
  <img src="./public/logo.png" alt="OpenFMV Logo" width="128" />
</p>

<p align="center">
  <mark><strong>This project is evolving quickly. Stay tuned.</strong></mark>
</p>

<p align="center">
  English · <a href="./README.zh-CN.md">简体中文</a> · <a href="./README.ja.md">日本語</a> · <a href="./README.ko.md">한국어</a>
</p>

OpenFMV is an AI Native interactive content editor for building interactive videos, branching narratives, interactive short dramas, and local playable story experiences.

At its core is the node-level FlowTimeline: each scene can carry media tracks plus interaction tracks for buttons, hotspots, pause gates, timed branches, and variable actions. The Blueprint graph remains the high-level map for connecting those interactive scenes, while local assets, preview, export, and AI-assisted creation stay inside a local-first Next.js + Electron desktop app. Projects, imported media, timeline data, and generated packages stay on your machine. There is no account system, hosted database, or cloud storage dependency.

![OpenFMV editor overview](./public/readme/openfmv-editor-overview.png)

## Product Highlights

<table>
  <tr>
    <td width="25%" valign="top">
      <img src="./public/readme/feature-readme-preview.png" alt="Node interaction timeline" width="100%" />
      <br />
      <strong>Node interaction tracks</strong><br />
      Layer media with buttons, hotspots, pause gates, timed branches, and variable actions inside each scene.
    </td>
    <td width="25%" valign="top">
      <img src="./public/readme/feature-readme-blueprint.png" alt="Visual story map" width="100%" />
      <br />
      <strong>Visual story map</strong><br />
      Connect interactive scenes with a readable blueprint for branches, outputs, and story flow.
    </td>
    <td width="25%" valign="top">
      <img src="./public/readme/feature-readme-assets.png" alt="Local asset management" width="100%" />
      <br />
      <strong>Local asset library</strong><br />
      Import video, image, audio, and text assets into local project folders.
    </td>
    <td width="25%" valign="top">
      <img src="./public/readme/feature-readme-export.png" alt="Local export" width="100%" />
      <br />
      <strong>Local export</strong><br />
      Package playable interactive content while keeping media references local.
    </td>
  </tr>
</table>

## What You Can Build

- Interactive videos and branching narrative prototypes
- Interactive short drama scenes with choice-driven playback
- Local playable story packages for demos, reviews, and experiments
- AI-assisted narrative workflows that still keep project data local

## Creative Workflow

1. Create or open a local project from the project workspace.
2. Import source media into the local asset library.
3. Edit each scene in `/nodes` with FlowTimeline media and interaction tracks.
4. Add buttons, hotspots, pause gates, timed branches, and variable actions as interaction clips.
5. Connect scenes in the `/editor` blueprint graph when the story flow needs branching structure.
6. Preview interactive playback and export a local playable package when the story is ready to share or test.

## Product Tour

### Interactive Playback Preview

Preview how a viewer moves through the story. This is where button choices, scene transitions, and interactive playback can be checked in context.

![OpenFMV playback preview](./public/readme/openfmv-play-preview.png)

### Local Project Workspace

Start from local drafts, project templates, and recent work. OpenFMV is designed around local project files rather than hosted workspaces.

![OpenFMV project workspace](./public/readme/openfmv-projects.jpg)

### Story Blueprint Editor

The editor is the high-level story map. It is responsible for story flow, node relationships, branch outputs, node prompts, and scene metadata.

![OpenFMV story blueprint](./public/readme/openfmv-editor-overview.png)

### AI Native Configuration

OpenFMV is built to work with local AI terminals and model services. The AI layer is intended to assist writing, ideation, and editing workflows while leaving project storage local.

![OpenFMV AI configuration](./public/readme/openfmv-aiconfig-preview.jpg)

### Visual Story Presets

Preset content can provide a quick starting point for interactive story experiments and visual direction.

![OpenFMV default story preset](./public/readme/default-story-preset.png)

## Core Capabilities

- **FlowTimeline scene editing:** Edit each node as an independent timeline with media and interaction tracks.
- **Interaction clips:** Add buttons, hotspots, pause gates, timed branches, and variable actions through timeline clips.
- **Blueprint graph editing:** Connect interactive nodes into nonlinear story flow with handles, edges, and branch outputs.
- **Local media workflow:** Copy imported files into local project asset folders and preserve those references through export.
- **AI-assisted creation:** Configure local AI engines and use assistant workflows without introducing user accounts or cloud sync.
- **Desktop-first experience:** Run as a packaged Electron app backed by a local Next.js standalone service.

## Current Boundaries

OpenFMV is intentionally local-first. The current product does not include login, multi-user collaboration, cloud sync, cloud databases, hosted media libraries, or one-click publishing to third-party platforms.

AI features are assistive. The project does not yet provide fully automated end-to-end generation of scripts, storyboards, visual assets, and interaction logic.

Export focuses on local playable packages and desktop app distribution workflows. Full Windows EXE story packaging is not part of the current product scope.

## Tech Stack

- **Framework:** Next.js 16 App Router, React, TypeScript
- **Desktop shell:** Electron
- **Graph editing:** React Flow
- **State:** Zustand and local browser storage
- **Styling:** Tailwind CSS with `openfmv-*` design tokens
- **Persistence:** Local OpenFMV project JSON files plus copied local assets
- **Runtime:** Shared graph runtime used by preview and export

## Quickstart

### Requirements

- Node.js 20 or newer
- npm
- Windows is the primary desktop packaging target today

### Install

```bash
npm install
```

### Start the web app

```bash
npm run dev
```

Then open `http://localhost:3000`.

### Start the desktop app in development

```bash
npm run desktop:dev
```

### Build the Next.js app

```bash
npm run build
```

### Build a packaged desktop app

```bash
npm run package:desktop
```

The packaged desktop app starts a local Next.js standalone service in the background and opens the main interface after the service is ready. If the local service cannot be reached, OpenFMV shows a diagnostic error page with the runtime log path.

## Common Commands

```bash
npm run dev
npm run desktop
npm run desktop:dev
npm run desktop:standalone
npm run build
npm run package:desktop
npm run lint
npm run test:run
```

Run a single test file:

```bash
npx vitest path/to/test.test.ts
```

Run a single named test:

```bash
npx vitest path/to/test.test.ts -t "test name"
```

## Project Structure

```text
app/
  _components/          React components
    editor/             Blueprint editor UI
    local/              Desktop/local project UI
    nodes/              React Flow node components
    player/             Player and preview UI
    ui/                 Shared UI primitives
  _features/
    node-timeline/      NodeTimeline v2 schema, UI, commands, snapping, playback
  _hooks/               React hooks
  _store/               Zustand stores
  _types/               Shared TypeScript types
  _utils/               Runtime and graph utilities
  api/                  Local Next.js API routes
  editor/               Blueprint editor route
  nodes/                Node-level timeline editor route
  play/[id]/            Player route
  projects/             Project workspace route
electron/
  main.js               Electron main process and local service bootstrap
  preload.js            Electron preload bridge
  exporter.js           Local playable package exporter
public/
  readme/               README screenshots
shared/
  runtimeCore.mjs       Shared runtime used by player and exporter
messages/
  *.json                next-intl locale files
__tests__/
  unit/                 Vitest unit tests
```

## Project Files

OpenFMV stores projects as local project files and local copied assets. Imported media should live in project asset folders and should be referenced from `node.data.timeline` rather than legacy node-level media fields.

The node timeline model is the primary media and interaction model:

- Media tracks contain video, image, and audio clips.
- Interaction tracks contain button, hotspot, pause gate, text, branch, and variable clips.
- Runtime preview and export compile from the timeline model.

## Export and Packaging

OpenFMV export rewrites timeline clip media paths into a local playable package. Timeline clip `src` and `poster` values are copied and rewritten during export.

Desktop packaging uses Electron Builder. Generated executables, installers, and unpacked app folders are written to `dist/` and ignored by git.

Desktop icons are generated from `public/logo.png` before packaging:

```text
build/icons/icon.ico
build/icons/icon.png
```

Windows desktop releases use NSIS auto-update artifacts. Increment `package.json` `version`, then run `npm run package:desktop` for a local release build or `npm run release:desktop` to publish to the configured provider. The release set should include the installer, `latest.yml`, and the installer `.blockmap` file. See `DESKTOP_AUTO_UPDATE.md` for update hosting and unsigned internal build notes.

## Development Notes

- Keep timeline behavior in `app/_features/node-timeline/`.
- Keep shared runtime behavior in `shared/runtimeCore.mjs`.
- Keep player UI in `app/_components/player/`.
- Keep local desktop UI in `app/_components/local/`.
- Do not add hosted backend, user accounts, cloud storage, or sync features unless the product scope changes explicitly.

## Contributing

OpenFMV is still moving quickly. Keep changes focused, local-first, and aligned with the timeline-based architecture.

## Acknowledgements

OpenFMV is built with Next.js, Electron, React Flow, Zustand, Tailwind CSS, and the broader open-source JavaScript ecosystem.

OpenFMV also thanks these open-source projects for inspiration and reference:

- [OpenCut-app/OpenCut](https://github.com/OpenCut-app/OpenCut)
- [nexu-io/open-design](https://github.com/nexu-io/open-design)

## License

MIT
