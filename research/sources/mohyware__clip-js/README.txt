## Overview

This is an online video editor built with nextjs, remotion for real-time preview and ffmpeg (web assembly port) for high-quality render.

## Features

- 🎞️ Real-time Preview: See immediate previews of edits.
- 🧰 Render with ffmpeg (web assembly port) with various options supports up to 1080p export.
- 🕹️ Interactive Timeline Editor: Precisely arrange, trim, and control media through a custom-built timeline.
- ✂️ Element Utilities: Easily split, duplicate, and manage individual media layers.
- 🖼️ Flexible Media Support: Import and mix videos, audio tracks, images, and text elements seamlessly.
- 🛠️ Advanced Element Controls: Adjust properties like position, opacity, z-index and volume per element.
- ⌨️ Keyboard Shortcuts: Quickly play, mute, move in time with arrows, split, duplicate, etc .

![Alt Text](/images/image.png)

## Installation

Clone the repo, install dependencies:

```bash
npm install
```
Then run the development server:
```bash
npm run dev
```
Or build and start in production mode:

```bash
npm run build
npm start
```

Alternatively, use Docker:

```bash
# Build the Docker image
docker build -t clipjs .

# Run the container
docker run -p 3000:3000 clipjs
```
Then navigate to [http://localhost:3000](http://localhost:3000)

## TODOs

Prioritized tasks are listed in [TODO.md](./TODO.md). 

contributions are welcomed!
