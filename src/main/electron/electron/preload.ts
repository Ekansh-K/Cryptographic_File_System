import { contextBridge, ipcRenderer } from 'electron'

// Expose protected methods that allow the renderer process to use
// the ipcRenderer without exposing the entire object
contextBridge.exposeInMainWorld('electronAPI', {
  getAppVersion: () => ipcRenderer.invoke('get-app-version'),
  getPlatform: () => ipcRenderer.invoke('get-platform'),
  selectFile: (options?: any) => ipcRenderer.invoke('select-file', options),
  selectDirectory: () => ipcRenderer.invoke('select-directory'),
  setTheme: (theme: string) => ipcRenderer.invoke('set-theme', theme),
  getTheme: () => ipcRenderer.invoke('get-theme'),
  
  // Menu event listeners
  onMenuNewContainer: (callback: () => void) => {
    ipcRenderer.on('menu-new-container', callback)
  },
  onMenuOpenContainer: (callback: () => void) => {
    ipcRenderer.on('menu-open-container', callback)
  },
  onMenuAbout: (callback: () => void) => {
    ipcRenderer.on('menu-about', callback)
  },
})

// Expose EFS-specific APIs
contextBridge.exposeInMainWorld('efsApi', {
  // System information
  getSystemCapabilities: () => ipcRenderer.invoke('get-system-capabilities'),
  getEnhancedSystemStatus: () => ipcRenderer.invoke('get-enhanced-system-status'),
  getCpuInfo: () => ipcRenderer.invoke('get-cpu-info'),
  
  // Container operations
  mountContainer: (containerId: string, password: string, mountPoint: string) => 
    ipcRenderer.invoke('mount-container', containerId, password, mountPoint),
  unmountContainer: (containerId: string) => 
    ipcRenderer.invoke('unmount-container', containerId),
  getMountStatus: (containerId: string) => 
    ipcRenderer.invoke('get-mount-status', containerId),
  
  // Hardware detection
  checkHardwareAcceleration: () => ipcRenderer.invoke('check-hardware-acceleration'),
  checkNativeDrivers: () => ipcRenderer.invoke('check-native-drivers'),
})

// Types for the exposed API
declare global {
  interface Window {
    electronAPI: {
      getAppVersion: () => Promise<string>
      getPlatform: () => Promise<string>
      selectFile: (options?: any) => Promise<string | null>
      selectDirectory: () => Promise<string | null>
      setTheme: (theme: string) => Promise<void>
      getTheme: () => Promise<string>
      onMenuNewContainer: (callback: () => void) => void
      onMenuOpenContainer: (callback: () => void) => void
      onMenuAbout: (callback: () => void) => void
    }
    efsApi: {
      getSystemCapabilities: () => Promise<any>
      getEnhancedSystemStatus: () => Promise<any>
      getCpuInfo: () => Promise<any>
      mountContainer: (containerId: string, password: string, mountPoint: string) => Promise<any>
      unmountContainer: (containerId: string) => Promise<boolean>
      getMountStatus: (containerId: string) => Promise<any>
      checkHardwareAcceleration: () => Promise<boolean>
      checkNativeDrivers: () => Promise<boolean>
    }
  }
}