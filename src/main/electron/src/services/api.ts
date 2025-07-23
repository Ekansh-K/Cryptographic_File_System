import axios, { AxiosInstance, AxiosResponse } from 'axios'

// API Configuration
const API_BASE_URL = process.env.NODE_ENV === 'development' 
  ? 'http://localhost:8080/api' 
  : 'http://localhost:8080/api'

// Check if we should use mock API (when backend is not available)
const USE_MOCK_API = false // Mock API disabled - using real backend

// Create axios instance
const api: AxiosInstance = axios.create({
  baseURL: API_BASE_URL,
  timeout: 30000,
  headers: {
    'Content-Type': 'application/json',
  },
})

// Request interceptor
api.interceptors.request.use(
  (config) => {
    // Add auth token if available
    const token = localStorage.getItem('auth_token')
    if (token) {
      config.headers.Authorization = `Bearer ${token}`
    }
    return config
  },
  (error) => {
    return Promise.reject(error)
  }
)

// Response interceptor
api.interceptors.response.use(
  (response: AxiosResponse) => {
    return response
  },
  (error) => {
    if (error.response?.status === 401) {
      // Handle unauthorized access
      localStorage.removeItem('auth_token')
      // Redirect to login if needed
    }
    return Promise.reject(error)
  }
)

// API Types
export interface Container {
  id: string
  name: string
  path: string
  size: number
  status: 'mounted' | 'unmounted' | 'locked' | 'error'
  createdAt: string
  lastAccessed: string
  encrypted: boolean
  steganographic: boolean
}

export interface ContainerConfig {
  name: string
  size: number
  password: string
  encryptionAlgorithm: string
  steganographic: boolean
  carrierFile?: string
}

export interface SharePackage {
  id: string
  name: string
  files: string[]
  createdAt: string
  expiresAt?: string
  accessCount: number
  maxAccess?: number
}

// Enhanced system information types for OSHI integration
export interface CpuFeaturesDto {
  hasAesNi: boolean
  hasAvx: boolean
  hasAvx2: boolean
  hasRdrand: boolean
  hasRdseed: boolean
  hasVaes: boolean
}

export interface CpuCacheDto {
  l1Cache: number
  l2Cache: number
  l3Cache: number
}

export interface CpuInfoDto {
  modelName: string
  vendor: string
  physicalCores: number
  logicalCores: number
  baseFrequency: number
  maxFrequency: number
  features: CpuFeaturesDto
  cache: CpuCacheDto
  currentUsage: number
}

export interface MemoryInfoDto {
  totalPhysical: number
  availablePhysical: number
  usedPhysical: number
  totalVirtual: number
  availableVirtual: number
  usedVirtual: number
  usagePercentage: number
}

export interface OperatingSystemDto {
  name: string
  version: string
  buildNumber: string
  architecture: string
  isVirtualMachine: boolean
  bootTime: number
  processCount: number
  threadCount: number
}

export interface DiskInfoDto {
  name: string
  model: string
  serial: string
  size: number
  reads: number
  writes: number
  readBytes: number
  writeBytes: number
  transferTime: number
}

export interface NetworkInterfaceDto {
  name: string
  displayName: string
  macAddress: string
  ipv4Addresses: string[]
  ipv6Addresses: string[]
  bytesReceived: number
  bytesSent: number
  packetsReceived: number
  packetsSent: number
  speed: number
  isUp: boolean
}

export interface SystemStatus {
  // Existing fields for backward compatibility
  version: string
  uptime: number
  containersCount: number
  mountedCount: number
  totalSize: number
  availableSpace: number
  
  // New OSHI-based fields
  cpu?: CpuInfoDto
  memory?: MemoryInfoDto
  operatingSystem?: OperatingSystemDto
  disks?: DiskInfoDto[]
  networkInterfaces?: NetworkInterfaceDto[]
}

// API Methods
export const containerAPI = {
  // Get all containers
  getContainers: (): Promise<Container[]> =>
    api.get('/containers').then(res => res.data),

  // Get container by ID
  getContainer: (id: string): Promise<Container> =>
    api.get(`/containers/${id}`).then(res => res.data),

  // Create new container
  createContainer: (config: ContainerConfig): Promise<Container> =>
    api.post('/containers', config).then(res => res.data),

  // Mount container
  mountContainer: (id: string, password: string, mountPoint?: string): Promise<void> =>
    api.post(`/containers/${id}/mount`, { password, mountPoint }).then(res => res.data),

  // Unmount container
  unmountContainer: (id: string): Promise<void> =>
    api.post(`/containers/${id}/unmount`).then(res => res.data),

  // Delete container
  deleteContainer: (id: string): Promise<void> =>
    api.delete(`/containers/${id}`).then(res => res.data),

  // Check container integrity
  checkIntegrity: (id: string): Promise<{ valid: boolean; issues: string[] }> =>
    api.post(`/containers/${id}/integrity-check`).then(res => res.data),

  // Resize container
  resizeContainer: (id: string, newSize: number): Promise<void> =>
    api.post(`/containers/${id}/resize`, { size: newSize }).then(res => res.data),
}

// Sharing Types
export interface SharedContainer {
  id: string
  containerId: string
  containerName: string
  recipientUsername: string
  permissions: SharePermission[]
  createdAt: string
  expiresAt?: string
  status: ShareStatus
  accessCount: number
  lastAccessed?: string
}

export interface ReceivedShare {
  id: string
  containerId: string
  containerName: string
  senderUsername: string
  permissions: SharePermission[]
  createdAt: string
  expiresAt?: string
  status: ShareStatus
  message?: string
}

export interface ShareConfig {
  recipientUsername: string
  permissions: SharePermission[]
  expiresAt?: string
  message?: string
  maxAccess?: number
}

export enum SharePermission {
  READ = 'read',
  WRITE = 'write',
  SHARE = 'share'
}

export enum ShareStatus {
  PENDING = 'pending',
  ACCEPTED = 'accepted',
  DECLINED = 'declined',
  REVOKED = 'revoked',
  EXPIRED = 'expired'
}

export const sharingAPI = {
  // Get all shares (legacy)
  getShares: (): Promise<SharePackage[]> =>
    api.get('/shares').then(res => res.data),

  // Create share (legacy)
  createShare: (containerId: string, files: string[], config: any): Promise<SharePackage> =>
    api.post('/shares', { containerId, files, ...config }).then(res => res.data),

  // Revoke share (legacy)
  revokeShare: (shareId: string): Promise<void> =>
    api.delete(`/shares/${shareId}`).then(res => res.data),

  // Share container with user by username
  shareWithUser: async (containerId: string, username: string, config: ShareConfig): Promise<SharedContainer> => {
    const response = await api.post('/shares/user', { 
      containerId, 
      username, 
      ...config 
    })
    return response.data
  },

  // Get containers shared by current user
  getMyShares: async (): Promise<SharedContainer[]> => {
    const response = await api.get('/shares/my-shares')
    return response.data
  },

  // Get shares received by current user
  getReceivedShares: async (): Promise<ReceivedShare[]> => {
    const response = await api.get('/shares/received')
    return response.data
  },

  // Accept a received share
  acceptShare: async (shareId: string): Promise<void> => {
    await api.post(`/shares/${shareId}/accept`)
  },

  // Decline a received share
  declineShare: async (shareId: string): Promise<void> => {
    await api.post(`/shares/${shareId}/decline`)
  },

  // Revoke a share (for share owner)
  revokeUserShare: async (shareId: string): Promise<void> => {
    await api.delete(`/shares/${shareId}`)
  },

  // Get share details
  getShareDetails: async (shareId: string): Promise<SharedContainer | ReceivedShare> => {
    const response = await api.get(`/shares/${shareId}`)
    return response.data
  },

  // Update share permissions
  updateSharePermissions: async (shareId: string, permissions: SharePermission[]): Promise<void> => {
    await api.patch(`/shares/${shareId}/permissions`, { permissions })
  },

  // Extend share expiration
  extendShareExpiration: async (shareId: string, expiresAt: string): Promise<void> => {
    await api.patch(`/shares/${shareId}/expiration`, { expiresAt })
  },

  // Get sharing statistics
  getSharingStats: async (): Promise<{
    totalShared: number
    totalReceived: number
    activeShares: number
    pendingShares: number
  }> => {
    const response = await api.get('/shares/stats')
    return response.data
  },

  // Search shareable containers
  getShareableContainers: async (): Promise<Container[]> => {
    const response = await api.get('/shares/containers')
    return response.data
  },

  // Enhanced user search with autocomplete for sharing
  searchUsersForSharing: async (query: string, limit: number = 5): Promise<User[]> => {
    const response = await api.get('/users/search/sharing', {
      params: { q: query, limit }
    })
    return response.data.users || []
  },

  // Validate if user can be shared with
  validateShareRecipient: async (username: string, containerId: string): Promise<{
    valid: boolean
    reason?: string
    suggestions?: string[]
  }> => {
    try {
      const response = await api.post('/shares/validate-recipient', {
        username,
        containerId
      })
      return response.data
    } catch (error: any) {
      return {
        valid: false,
        reason: error.response?.data?.message || 'Validation failed'
      }
    }
  },

  // Get sharing notifications
  getNotifications: async (unreadOnly: boolean = false): Promise<ShareNotification[]> => {
    const response = await api.get('/shares/notifications', {
      params: { unreadOnly }
    })
    return response.data
  },

  // Mark notification as read
  markNotificationRead: async (notificationId: string): Promise<void> => {
    await api.patch(`/shares/notifications/${notificationId}/read`)
  },

  // Mark all notifications as read
  markAllNotificationsRead: async (): Promise<void> => {
    await api.patch('/shares/notifications/read-all')
  },

  // Get notification preferences
  getNotificationPreferences: async (): Promise<NotificationPreferences> => {
    const response = await api.get('/shares/notifications/preferences')
    return response.data
  },

  // Update notification preferences
  updateNotificationPreferences: async (preferences: Partial<NotificationPreferences>): Promise<void> => {
    await api.patch('/shares/notifications/preferences', preferences)
  },

  // Get real-time sharing status updates
  getShareStatusUpdates: async (shareIds: string[]): Promise<{
    [shareId: string]: {
      status: ShareStatus
      lastActivity: string
      accessCount: number
    }
  }> => {
    const response = await api.post('/shares/status-updates', { shareIds })
    return response.data
  },

  // Bulk operations for shares
  bulkAcceptShares: async (shareIds: string[]): Promise<{
    successful: string[]
    failed: { shareId: string; error: string }[]
  }> => {
    const response = await api.post('/shares/bulk-accept', { shareIds })
    return response.data
  },

  bulkDeclineShares: async (shareIds: string[]): Promise<{
    successful: string[]
    failed: { shareId: string; error: string }[]
  }> => {
    const response = await api.post('/shares/bulk-decline', { shareIds })
    return response.data
  },

  bulkRevokeShares: async (shareIds: string[]): Promise<{
    successful: string[]
    failed: { shareId: string; error: string }[]
  }> => {
    const response = await api.post('/shares/bulk-revoke', { shareIds })
    return response.data
  },

  // Audit logging endpoints
  logAuditEvent: async (event: {
    type: string
    shareId: string
    containerId: string
    details: Record<string, any>
    timestamp: string
  }): Promise<void> => {
    await api.post('/shares/audit/log', event)
  },

  getAuditLogs: async (filter: {
    shareId?: string
    userId?: string
    containerId?: string
    eventType?: string
    startDate?: string
    endDate?: string
    page?: number
    limit?: number
  } = {}): Promise<{
    events: any[]
    total: number
    page: number
    limit: number
  }> => {
    const response = await api.get('/shares/audit/logs', { params: filter })
    return response.data
  },

  getShareAuditTrail: async (shareId: string): Promise<any[]> => {
    const response = await api.get(`/shares/${shareId}/audit`)
    return response.data
  },

  // Enhanced error handling wrapper
  handleSharingError: (error: any): SharingError => {
    const status = error.response?.status
    const message = error.response?.data?.message || 'Sharing operation failed'
    const details = error.response?.data?.details

    let type: SharingErrorType
    let retryable = false

    switch (status) {
      case 404:
        if (message.toLowerCase().includes('container')) {
          type = SharingErrorType.CONTAINER_NOT_FOUND
        } else if (message.toLowerCase().includes('user')) {
          type = SharingErrorType.USER_NOT_FOUND
        } else {
          type = SharingErrorType.USER_NOT_FOUND
        }
        break
      case 403:
        type = SharingErrorType.INSUFFICIENT_PERMISSIONS
        break
      case 409:
        if (message.includes('already exists')) {
          type = SharingErrorType.SHARE_ALREADY_EXISTS
        } else if (message.includes('limit')) {
          type = SharingErrorType.SHARE_LIMIT_EXCEEDED
        } else {
          type = SharingErrorType.INVALID_PERMISSIONS
        }
        break
      case 410:
        type = SharingErrorType.SHARE_EXPIRED
        break
      case 423:
        type = SharingErrorType.CONTAINER_NOT_ACCESSIBLE
        break
      case 503:
        type = SharingErrorType.SHARING_DISABLED
        retryable = true
        break
      default:
        type = SharingErrorType.INSUFFICIENT_PERMISSIONS
        retryable = status >= 500
    }

    return {
      type,
      message,
      details,
      retryable
    }
  }
}

export const systemAPI = {
  // Get system status
  getStatus: (): Promise<SystemStatus> =>
    api.get('/system/status').then(res => res.data),

  // Get system logs
  getLogs: (level?: string, limit?: number): Promise<any[]> =>
    api.get('/system/logs', { params: { level, limit } }).then(res => res.data),
}

// User Authentication Types
export interface User {
  id: string
  username: string
  email?: string
  createdAt: string
  lastLogin: string
  isActive: boolean
}

export interface UserCredentials {
  username: string
  password: string
  rememberMe?: boolean
}

export interface NewUserData {
  username: string
  password: string
  confirmPassword: string
  email?: string
}

export interface AuthResult {
  user: User
  token: string
  refreshToken: string
  expiresAt: string
}

export interface LoginResponse {
  success: boolean
  user?: User
  token?: string
  refreshToken?: string
  expiresAt?: string
  error?: string
  requiresRegistration?: boolean
}

export interface RegistrationResponse {
  success: boolean
  user?: User
  token?: string
  error?: string
}

export interface UserSearchResult {
  users: User[]
  total: number
  page: number
  limit: number
}

// Authentication Error Types
export enum AuthErrorType {
  INVALID_CREDENTIALS = 'invalid_credentials',
  USER_NOT_FOUND = 'user_not_found',
  USERNAME_TAKEN = 'username_taken',
  WEAK_PASSWORD = 'weak_password',
  SESSION_EXPIRED = 'session_expired',
  ACCOUNT_LOCKED = 'account_locked',
  VALIDATION_ERROR = 'validation_error',
  SERVER_ERROR = 'server_error'
}

export interface AuthError {
  type: AuthErrorType
  message: string
  details?: any
  retryable: boolean
}

// Sharing Error Types
export enum SharingErrorType {
  USER_NOT_FOUND = 'user_not_found',
  CONTAINER_NOT_FOUND = 'container_not_found',
  INSUFFICIENT_PERMISSIONS = 'insufficient_permissions',
  SHARE_EXPIRED = 'share_expired',
  SHARE_LIMIT_EXCEEDED = 'share_limit_exceeded',
  SHARE_ALREADY_EXISTS = 'share_already_exists',
  INVALID_PERMISSIONS = 'invalid_permissions',
  CONTAINER_NOT_ACCESSIBLE = 'container_not_accessible',
  SHARING_DISABLED = 'sharing_disabled'
}

export interface SharingError {
  type: SharingErrorType
  message: string
  details?: any
  retryable: boolean
}

// Notification Types
export interface ShareNotification {
  id: string
  type: 'share_received' | 'share_accepted' | 'share_declined' | 'share_revoked' | 'share_expired'
  shareId: string
  fromUsername?: string
  toUsername?: string
  containerName: string
  message?: string
  createdAt: string
  read: boolean
}

export interface NotificationPreferences {
  shareReceived: boolean
  shareAccepted: boolean
  shareDeclined: boolean
  shareRevoked: boolean
  shareExpired: boolean
  emailNotifications: boolean
  pushNotifications: boolean
}

// User Authentication API
export const userAPI = {
  // Register new user
  register: async (userData: NewUserData): Promise<RegistrationResponse> => {
    try {
      const response = await api.post('/auth/register', userData)
      return {
        success: true,
        user: response.data.user,
        token: response.data.token
      }
    } catch (error: any) {
      return {
        success: false,
        error: error.response?.data?.message || 'Registration failed'
      }
    }
  },

  // Login user
  login: async (credentials: UserCredentials): Promise<LoginResponse> => {
    try {
      const response = await api.post('/auth/login', credentials)
      return {
        success: true,
        user: response.data.user,
        token: response.data.token,
        refreshToken: response.data.refreshToken,
        expiresAt: response.data.expiresAt
      }
    } catch (error: any) {
      const status = error.response?.status
      const message = error.response?.data?.message || 'Login failed'
      
      return {
        success: false,
        error: message,
        requiresRegistration: status === 404
      }
    }
  },

  // Logout user
  logout: async (): Promise<{ success: boolean; error?: string }> => {
    try {
      await api.post('/auth/logout')
      return { success: true }
    } catch (error: any) {
      return {
        success: false,
        error: error.response?.data?.message || 'Logout failed'
      }
    }
  },

  // Get current user
  getCurrentUser: async (): Promise<User> => {
    const response = await api.get('/auth/me')
    return response.data
  },

  // Refresh authentication token
  refreshToken: async (): Promise<AuthResult> => {
    const response = await api.post('/auth/refresh')
    return response.data
  },

  // Search users by username (for sharing features)
  searchUsers: async (query: string, limit: number = 10, page: number = 1): Promise<UserSearchResult> => {
    const response = await api.get('/users/search', { 
      params: { q: query, limit, page } 
    })
    return response.data
  },

  // Get user by username
  getUserByUsername: async (username: string): Promise<User | null> => {
    try {
      const response = await api.get(`/users/username/${username}`)
      return response.data
    } catch (error: any) {
      if (error.response?.status === 404) {
        return null
      }
      throw error
    }
  },

  // Validate username availability
  checkUsernameAvailability: async (username: string): Promise<{ available: boolean }> => {
    const response = await api.get('/auth/check-username', { 
      params: { username } 
    })
    return response.data
  },

  // Change password
  changePassword: async (currentPassword: string, newPassword: string): Promise<{ success: boolean; error?: string }> => {
    try {
      await api.post('/auth/change-password', {
        currentPassword,
        newPassword
      })
      return { success: true }
    } catch (error: any) {
      return {
        success: false,
        error: error.response?.data?.message || 'Password change failed'
      }
    }
  },

  // Validate session
  validateSession: async (): Promise<{ valid: boolean; user?: User }> => {
    try {
      const response = await api.get('/auth/validate')
      return {
        valid: true,
        user: response.data.user
      }
    } catch (error: any) {
      return { valid: false }
    }
  }
}

export default api