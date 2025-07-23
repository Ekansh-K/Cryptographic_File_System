import { credentialStorage } from './credentialStorage'
// JWT functionality implemented with Web Crypto API

// Types for session management
export interface User {
  id: string
  username: string
  createdAt: Date
  lastLogin: Date
}

export interface AuthResult {
  user: User
  token: string
  refreshToken: string
  expiresAt: Date
}

export interface Session {
  id: string
  userId: string
  username: string
  token: string
  refreshToken: string
  createdAt: Date
  expiresAt: Date
  lastActivity: Date
}

export interface SessionConfig {
  tokenExpirationMinutes: number
  refreshTokenExpirationDays: number
  maxConcurrentSessions: number
  autoRefreshThresholdMinutes: number
}

/**
 * Session management service with JWT token handling
 * Manages user sessions, token lifecycle, and automatic refresh
 */
export class SessionManager {
  private currentSession: Session | null = null
  private refreshTimer: NodeJS.Timeout | null = null
  private readonly config: SessionConfig

  private readonly SESSION_STORAGE_KEY = 'efs_session'
  private readonly REFRESH_TOKEN_KEY = 'efs_refresh_token'

  constructor(config?: Partial<SessionConfig>) {
    this.config = {
      tokenExpirationMinutes: 15,
      refreshTokenExpirationDays: 7,
      maxConcurrentSessions: 3,
      autoRefreshThresholdMinutes: 5,
      ...config
    }
  }

  /**
   * Create a new session after successful authentication
   */
  async createSession(user: User, rememberMe: boolean = false): Promise<AuthResult> {
    try {
      const now = new Date()
      const tokenExpiration = new Date(now.getTime() + this.config.tokenExpirationMinutes * 60 * 1000)
      const refreshExpiration = new Date(now.getTime() + this.config.refreshTokenExpirationDays * 24 * 60 * 60 * 1000)

      // Generate tokens (simplified - in production, use proper JWT library)
      const token = this.generateToken(user, tokenExpiration)
      const refreshToken = this.generateRefreshToken(user, refreshExpiration)

      const session: Session = {
        id: this.generateSessionId(),
        userId: user.id,
        username: user.username,
        token,
        refreshToken,
        createdAt: now,
        expiresAt: tokenExpiration,
        lastActivity: now
      }

      // Store session
      this.currentSession = session
      this.storeSession(session, rememberMe)
      
      // Set up auto-refresh timer
      this.setupAutoRefresh()

      const authResult: AuthResult = {
        user,
        token,
        refreshToken,
        expiresAt: tokenExpiration
      }

      return authResult
    } catch (error) {
      throw new Error(`Failed to create session: ${error instanceof Error ? error.message : 'Unknown error'}`)
    }
  }

  /**
   * Restore session from storage on application startup
   */
  async restoreSession(): Promise<Session | null> {
    try {
      const sessionData = localStorage.getItem(this.SESSION_STORAGE_KEY)
      if (!sessionData) {
        return null
      }

      const session: Session = JSON.parse(sessionData)
      
      // Check if session is expired
      if (new Date() > new Date(session.expiresAt)) {
        // Try to refresh the session
        const refreshed = await this.refreshSession()
        return refreshed
      }

      // Verify session integrity
      const isValid = await this.validateSession(session)
      if (!isValid) {
        await this.clearSession()
        return null
      }

      this.currentSession = session
      this.setupAutoRefresh()
      
      return session
    } catch (error) {
      console.error('Failed to restore session:', error)
      await this.clearSession()
      return null
    }
  }

  /**
   * Refresh the current session using refresh token
   */
  async refreshSession(): Promise<Session | null> {
    try {
      if (!this.currentSession) {
        return null
      }

      const refreshToken = localStorage.getItem(this.REFRESH_TOKEN_KEY)
      if (!refreshToken) {
        return null
      }

      // Validate refresh token
      const isValidRefresh = this.validateRefreshToken(refreshToken)
      if (!isValidRefresh) {
        await this.clearSession()
        return null
      }

      // Create new session with extended expiration
      const now = new Date()
      const newExpiration = new Date(now.getTime() + this.config.tokenExpirationMinutes * 60 * 1000)
      const newToken = this.generateToken({
        id: this.currentSession.userId,
        username: this.currentSession.username,
        createdAt: new Date(),
        lastLogin: new Date()
      }, newExpiration)

      const updatedSession: Session = {
        ...this.currentSession,
        token: newToken,
        expiresAt: newExpiration,
        lastActivity: now
      }

      this.currentSession = updatedSession
      this.storeSession(updatedSession, true)
      this.setupAutoRefresh()

      return updatedSession
    } catch (error) {
      console.error('Failed to refresh session:', error)
      await this.clearSession()
      return null
    }
  }

  /**
   * Clear current session and logout
   */
  async clearSession(): Promise<void> {
    try {
      // Clear timers
      if (this.refreshTimer) {
        clearTimeout(this.refreshTimer)
        this.refreshTimer = null
      }

      // Clear storage
      localStorage.removeItem(this.SESSION_STORAGE_KEY)
      localStorage.removeItem(this.REFRESH_TOKEN_KEY)
      localStorage.removeItem('auth_token')

      // Clear current session
      this.currentSession = null
    } catch (error) {
      console.error('Failed to clear session:', error)
    }
  }

  /**
   * Get current session
   */
  getCurrentSession(): Session | null {
    return this.currentSession
  }

  /**
   * Check if user is authenticated
   */
  isAuthenticated(): boolean {
    if (!this.currentSession) {
      return false
    }

    // Check if session is expired
    return new Date() < new Date(this.currentSession.expiresAt)
  }

  /**
   * Get current user
   */
  getCurrentUser(): User | null {
    if (!this.currentSession) {
      return null
    }

    return {
      id: this.currentSession.userId,
      username: this.currentSession.username,
      createdAt: new Date(),
      lastLogin: new Date()
    }
  }

  /**
   * Update last activity timestamp
   */
  updateActivity(): void {
    if (this.currentSession) {
      this.currentSession.lastActivity = new Date()
      this.storeSession(this.currentSession, true)
    }
  }

  /**
   * Get session statistics
   */
  getSessionStats(): {
    isActive: boolean
    timeRemaining: number
    lastActivity: Date | null
    autoRefreshEnabled: boolean
  } {
    if (!this.currentSession) {
      return {
        isActive: false,
        timeRemaining: 0,
        lastActivity: null,
        autoRefreshEnabled: false
      }
    }

    const now = new Date()
    const expiresAt = new Date(this.currentSession.expiresAt)
    const timeRemaining = Math.max(0, expiresAt.getTime() - now.getTime())

    return {
      isActive: this.isAuthenticated(),
      timeRemaining,
      lastActivity: this.currentSession.lastActivity,
      autoRefreshEnabled: this.refreshTimer !== null
    }
  }

  /**
   * Generate session token (simplified JWT-like structure)
   */
  private generateToken(user: User, expiresAt: Date): string {
    const header = {
      alg: 'HS256',
      typ: 'JWT'
    }

    const payload = {
      sub: user.id,
      username: user.username,
      iat: Math.floor(Date.now() / 1000),
      exp: Math.floor(expiresAt.getTime() / 1000)
    }

    // In production, use proper JWT library with signing
    const headerB64 = btoa(JSON.stringify(header))
    const payloadB64 = btoa(JSON.stringify(payload))
    const signature = this.generateSignature(`${headerB64}.${payloadB64}`)

    return `${headerB64}.${payloadB64}.${signature}`
  }

  /**
   * Generate refresh token
   */
  private generateRefreshToken(user: User, expiresAt: Date): string {
    const tokenData = {
      userId: user.id,
      username: user.username,
      exp: expiresAt.getTime(),
      random: Math.random().toString(36)
    }

    return btoa(JSON.stringify(tokenData))
  }

  /**
   * Generate session ID
   */
  private generateSessionId(): string {
    return `session_${Date.now()}_${Math.random().toString(36).substring(2, 11)}`
  }

  /**
   * Generate signature for token (simplified)
   */
  private generateSignature(data: string): string {
    // In production, use proper HMAC with secret key
    return btoa(data).substring(0, 32)
  }

  /**
   * Validate session integrity
   */
  private async validateSession(session: Session): Promise<boolean> {
    try {
      // Check if user credentials still exist
      const hasCredentials = await credentialStorage.verifyIntegrity(session.username)
      if (!hasCredentials) {
        return false
      }

      // Validate token structure
      const tokenParts = session.token.split('.')
      if (tokenParts.length !== 3) {
        return false
      }

      // Validate token payload
      const payload = JSON.parse(atob(tokenParts[1]))
      if (payload.username !== session.username) {
        return false
      }

      return true
    } catch (error) {
      return false
    }
  }

  /**
   * Validate refresh token
   */
  private validateRefreshToken(refreshToken: string): boolean {
    try {
      const tokenData = JSON.parse(atob(refreshToken))
      const now = Date.now()
      
      return tokenData.exp > now && 
             tokenData.userId && 
             tokenData.username
    } catch (error) {
      return false
    }
  }

  /**
   * Store session in localStorage
   */
  private storeSession(session: Session, persistent: boolean): void {
    if (persistent) {
      localStorage.setItem(this.SESSION_STORAGE_KEY, JSON.stringify(session))
      localStorage.setItem(this.REFRESH_TOKEN_KEY, session.refreshToken)
      localStorage.setItem('auth_token', session.token)
    } else {
      // Store in sessionStorage for non-persistent sessions
      sessionStorage.setItem(this.SESSION_STORAGE_KEY, JSON.stringify(session))
      sessionStorage.setItem('auth_token', session.token)
    }
  }

  /**
   * Setup automatic token refresh
   */
  private setupAutoRefresh(): void {
    if (this.refreshTimer) {
      clearTimeout(this.refreshTimer)
    }

    if (!this.currentSession) {
      return
    }

    const now = new Date()
    const expiresAt = new Date(this.currentSession.expiresAt)
    const refreshThreshold = this.config.autoRefreshThresholdMinutes * 60 * 1000
    const timeUntilRefresh = expiresAt.getTime() - now.getTime() - refreshThreshold

    if (timeUntilRefresh > 0) {
      this.refreshTimer = setTimeout(async () => {
        await this.refreshSession()
      }, timeUntilRefresh)
    }
  }
}

// Export singleton instance
export const sessionManager = new SessionManager()