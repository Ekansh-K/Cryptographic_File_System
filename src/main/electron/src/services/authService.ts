import { credentialStorage } from './credentialStorage'
import { sessionManager, User, Session } from './sessionManager'

// Types for authentication
export interface LoginCredentials {
  username: string
  password: string
  rememberMe?: boolean
}

export interface RegistrationData {
  username: string
  password: string
  confirmPassword: string
}

export interface AuthenticationResult {
  success: boolean
  user?: User
  token?: string
  error?: string
  requiresRegistration?: boolean
}

export enum AuthErrorType {
  INVALID_CREDENTIALS = 'invalid_credentials',
  USER_NOT_FOUND = 'user_not_found',
  USERNAME_TAKEN = 'username_taken',
  WEAK_PASSWORD = 'weak_password',
  SESSION_EXPIRED = 'session_expired',
  PASSWORDS_DONT_MATCH = 'passwords_dont_match',
  CREDENTIAL_CORRUPTION = 'credential_corruption'
}

export interface AuthError {
  type: AuthErrorType
  message: string
  details?: any
  retryable: boolean
}

/**
 * Main authentication service that coordinates credential storage and session management
 */
export class AuthenticationService {
  private readonly MIN_PASSWORD_LENGTH = 8
  private readonly PASSWORD_COMPLEXITY_REGEX = /^(?=.*[a-z])(?=.*[A-Z])(?=.*\d)(?=.*[@$!%*?&])[A-Za-z\d@$!%*?&]/

  /**
   * Register a new user
   */
  async register(registrationData: RegistrationData): Promise<AuthenticationResult> {
    try {
      // Validate registration data
      const validation = this.validateRegistrationData(registrationData)
      if (!validation.valid) {
        return {
          success: false,
          error: validation.error
        }
      }

      // Check if username already exists
      const existingCredentials = await credentialStorage.retrieveCredentials(registrationData.username)
      if (existingCredentials) {
        return {
          success: false,
          error: 'Username already exists'
        }
      }

      // Store credentials securely
      await credentialStorage.storeCredentials(registrationData.username, registrationData.password)

      // Create user object
      const user: User = {
        id: await this.generateUserId(registrationData.username),
        username: registrationData.username,
        createdAt: new Date(),
        lastLogin: new Date()
      }

      // Create session
      const authResult = await sessionManager.createSession(user, false)

      return {
        success: true,
        user: authResult.user,
        token: authResult.token
      }
    } catch (error) {
      return {
        success: false,
        error: `Registration failed: ${error instanceof Error ? error.message : 'Unknown error'}`
      }
    }
  }

  /**
   * Login with username and password
   */
  async login(credentials: LoginCredentials): Promise<AuthenticationResult> {
    try {
      // Retrieve stored credentials
      const storedCredentials = await credentialStorage.retrieveCredentials(credentials.username)
      if (!storedCredentials) {
        return {
          success: false,
          error: 'User not found',
          requiresRegistration: true
        }
      }

      // Verify password
      const isValidPassword = await this.verifyPassword(credentials.password, storedCredentials.password)
      if (!isValidPassword) {
        return {
          success: false,
          error: 'Invalid credentials'
        }
      }

      // Create user object
      const user: User = {
        id: await this.generateUserId(credentials.username),
        username: credentials.username,
        createdAt: new Date(),
        lastLogin: new Date()
      }

      // Create session
      const authResult = await sessionManager.createSession(user, credentials.rememberMe || false)

      return {
        success: true,
        user: authResult.user,
        token: authResult.token
      }
    } catch (error) {
      return {
        success: false,
        error: `Login failed: ${error instanceof Error ? error.message : 'Unknown error'}`
      }
    }
  }

  /**
   * Logout current user
   */
  async logout(): Promise<void> {
    await sessionManager.clearSession()
  }

  /**
   * Check if user is currently authenticated
   */
  isAuthenticated(): boolean {
    return sessionManager.isAuthenticated()
  }

  /**
   * Get current authenticated user
   */
  getCurrentUser(): User | null {
    return sessionManager.getCurrentUser()
  }

  /**
   * Get current session
   */
  getCurrentSession(): Session | null {
    return sessionManager.getCurrentSession()
  }

  /**
   * Restore session on application startup
   */
  async restoreSession(): Promise<Session | null> {
    return await sessionManager.restoreSession()
  }

  /**
   * Refresh current session
   */
  async refreshSession(): Promise<Session | null> {
    return await sessionManager.refreshSession()
  }

  /**
   * Verify credential integrity
   */
  async verifyCredentialIntegrity(): Promise<boolean> {
    try {
      const currentUser = this.getCurrentUser()
      if (!currentUser) {
        return true // No user to verify
      }

      return await credentialStorage.verifyIntegrity(currentUser.username)
    } catch (error) {
      console.error('Credential integrity check failed:', error)
      return false
    }
  }

  /**
   * Get list of stored usernames
   */
  getStoredUsernames(): string[] {
    return credentialStorage.getStoredUsernames()
  }

  /**
   * Delete user credentials
   */
  async deleteUser(username: string): Promise<void> {
    await credentialStorage.deleteCredentials(username)
  }

  /**
   * Clear all stored credentials
   */
  clearAllCredentials(): void {
    credentialStorage.clearAllCredentials()
  }

  /**
   * Change user password
   */
  async changePassword(username: string, currentPassword: string, newPassword: string): Promise<AuthenticationResult> {
    try {
      // Verify current password
      const storedCredentials = await credentialStorage.retrieveCredentials(username)
      if (!storedCredentials) {
        return {
          success: false,
          error: 'User not found'
        }
      }

      const isValidCurrentPassword = await this.verifyPassword(currentPassword, storedCredentials.password)
      if (!isValidCurrentPassword) {
        return {
          success: false,
          error: 'Current password is incorrect'
        }
      }

      // Validate new password
      if (!this.isPasswordStrong(newPassword)) {
        return {
          success: false,
          error: 'New password does not meet security requirements'
        }
      }

      // Store new password
      await credentialStorage.storeCredentials(username, newPassword)

      return {
        success: true
      }
    } catch (error) {
      return {
        success: false,
        error: `Password change failed: ${error instanceof Error ? error.message : 'Unknown error'}`
      }
    }
  }

  /**
   * Get session statistics
   */
  getSessionStats() {
    return sessionManager.getSessionStats()
  }

  /**
   * Update session activity
   */
  updateActivity(): void {
    sessionManager.updateActivity()
  }

  /**
   * Validate registration data
   */
  private validateRegistrationData(data: RegistrationData): { valid: boolean; error?: string } {
    // Check username
    if (!data.username || data.username.trim().length < 3) {
      return { valid: false, error: 'Username must be at least 3 characters long' }
    }

    if (!/^[a-zA-Z0-9_-]+$/.test(data.username)) {
      return { valid: false, error: 'Username can only contain letters, numbers, underscores, and hyphens' }
    }

    // Check password
    if (!this.isPasswordStrong(data.password)) {
      return { 
        valid: false, 
        error: 'Password must be at least 8 characters long and contain uppercase, lowercase, number, and special character' 
      }
    }

    // Check password confirmation
    if (data.password !== data.confirmPassword) {
      return { valid: false, error: 'Passwords do not match' }
    }

    return { valid: true }
  }

  /**
   * Check if password meets strength requirements
   */
  private isPasswordStrong(password: string): boolean {
    return password.length >= this.MIN_PASSWORD_LENGTH && 
           this.PASSWORD_COMPLEXITY_REGEX.test(password)
  }

  /**
   * Verify password against stored password
   */
  private async verifyPassword(inputPassword: string, storedPassword: string): Promise<boolean> {
    // In a real implementation, you would hash the input password and compare
    // For now, we're doing direct comparison since we're storing plaintext
    // In production, use bcrypt or similar for password hashing
    return inputPassword === storedPassword
  }

  /**
   * Generate user ID from username
   */
  private async generateUserId(username: string): Promise<string> {
    const encoder = new TextEncoder()
    const data = encoder.encode(username)
    const hashBuffer = await crypto.subtle.digest('SHA-256', data)
    const hashArray = new Uint8Array(hashBuffer)
    const hash = Array.from(hashArray)
      .map(b => b.toString(16).padStart(2, '0'))
      .join('')
    return `user_${hash.substring(0, 16)}`
  }
}

// Export singleton instance
export const authService = new AuthenticationService()

// Types are already exported above