# EFS UI Application

Modern Electron + React + TypeScript management interface for the Encrypted File System.

## Features

- **Modern UI Framework**: Built with Electron, React 18, TypeScript, and Vite
- **Component Library**: Ant Design for professional UI components
- **Styling**: Tailwind CSS with custom design system
- **Theme Support**: Light/Dark/System theme switching
- **Responsive Design**: Works on different screen sizes
- **REST API Integration**: Communicates with Java backend
- **State Management**: Zustand for lightweight state management
- **Animations**: Framer Motion for smooth transitions

## Architecture

```
src/
├── components/          # Reusable UI components
│   └── layout/         # Layout components (Header, Sidebar)
├── contexts/           # React contexts (Theme)
├── pages/              # Main application pages
├── services/           # API services and utilities
├── App.tsx             # Main application component
├── main.tsx            # React entry point
└── index.css           # Global styles

electron/
├── main.ts             # Electron main process
└── preload.ts          # Secure IPC bridge
```

## Development

```bash
# Install dependencies
npm install

# Start development server
npm run dev

# Build for production
npm run build

# Type checking
npm run type-check

# Linting
npm run lint
```

## Pages

- **Dashboard**: System overview with statistics and quick actions
- **Containers**: Container management with mounting/unmounting controls
- **Settings**: Application preferences and security settings

## API Integration

The UI communicates with the Java backend through REST APIs:

- Container management (CRUD operations)
- System status and monitoring
- Security and sharing features
- Configuration management

## Security Features

- Secure IPC communication between processes
- Context isolation enabled
- No node integration in renderer
- External link protection
- Theme-aware security indicators

## Customization

The application supports extensive customization through:

- Theme system (light/dark/system)
- Tailwind CSS utility classes
- Ant Design theme tokens
- Custom CSS variables
- Responsive breakpoints