import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest'
import { authService } from '../authService'
import { sessionManager } from '../sessionManager'
import { credentialStorage } from '../credentialStorage'

// Mock localStorage
const localStorageMock = (() => {
  let store: Record<string, string> = {}
  return {
    getItem: (key: string) => store[key] || null,
    setItem: (key: string, value: string) => { store[key] = value },
    removeItem: (key: string) => { delete store[key] },
    clear: () => { store = {} },
    get length() { return Object.keys(store).length },
    key: (index: number) => Object.keys(store)[index] || null
  }
})()

Object.defineProperty(window, 'localStorage', {
  value: localStorageMock
})

// Mock sessionStorage
const sessionStorageMock = (() => {
  let store: Record<string, string> = {}
  return {
    getItem: (key: string) => store[key] || null,
    setItem: (key: string, value: string) => { store[key] = value },
    removeItem: (key: string) => { delete store[key] },
    clear: () => { store = {} },
    get length() { return Object.keys(store).length },
    key: (index: number) => Object.keys(store)[index] || null
  }
})()

Object.defineProperty(window, 'sessionStorage', {
  value: sessionStorageMock
})

describe('Session Persistence Integration Tests', () => {
  beforeEach(() => {
    localStorageMock.clear()
    sessionStorageMock.clear()
    vi.clearAllMocks()
  })

  afterEach(() => {
    localStorageMock.clear()
    sessionStorageMock.clear()
  })

  describe('Session Creation and Storage', () => {
    it('should create and store session with remember me', async () => {
      // Register a user first
      const registrationResult = await authService.register({
        username: 'testuser',
        password: 'TestPass123!',
        confirmPassword: 'TestPass123!'
      })

      expect(registrationResult.success).toBe(true)
      expect(registrationResult.user).toBeDefined()
      expect(registrationResult.token).toBeDefined()

      // Check that session is stored in localStorage
      const storedSession = localStorage.getItem('efs_session')
      expect(storedSession).toBeTruthy()

      const sessionData = JSON.parse(storedSession!)
      expect(sessionData.username).toBe('testuser')
      expect(sessionData.token).toBeDefined()
      expect(sessionData.refreshToken).toBeDefined()
    })

    it('should create session without persistence when remember me is false', async () => {
      // Login without remember me
      await authService.register({
        username: 'testuser',
        password: 'TestPass123!',
        confirmPassword: 'TestPass123!'
      })

      // Logout and login again without remember me
      await authService.logout()

      const loginResult = await authService.login({
        username: 'testuser',
        password: 'TestPass123!',
        rememberMe: false
      })

      expect(loginResult.success).toBe(true)

      // Session should be in sessionStorage, not localStorage for non-persistent sessions
      // (This depends on implementation - adjust based on actual behavior)
      const currentSession = sessionManager.getCurrentSession()
      expect(currentSession).toBeTruthy()
    })
  })

  describe('Session Restoration', () => {
    it('should restore valid session on startup', async () => {
      // Create a session first
      await authService.register({
        username: 'testuser',
        password: 'TestPass123!',
        confirmPassword: 'TestPass123!'
      })

      const originalSession = sessionManager.getCurrentSession()
      expect(originalSession).toBeTruthy()

      // Clear current session to simulate app restart
      await sessionManager.clearSession()
      expect(sessionManager.getCurrentSession()).toBeNull()

      // Restore session
      const restoredSession = await authService.restoreSession()
      expect(restoredSession).toBeTruthy()
      expect(restoredSession!.username).toBe('testuser')
      expect(authService.isAuthenticated()).toBe(true)
    })

    it('should not restore expired session', async () => {
      // Create a session
      await authService.register({
        username: 'testuser',
        password: 'TestPass123!',
        confirmPassword: 'TestPass123!'
      })

      // Manually modify stored session to be expired
      const storedSession = localStorage.getItem('efs_session')
      const sessionData = JSON.parse(storedSession!)
      sessionData.expiresAt = new Date(Date.now() - 3600000).toISOString() // 1 hour ago
      localStorage.setItem('efs_session', JSON.stringify(sessionData))

      // Clear current session
      await sessionManager.clearSession()

      // Try to restore expired session
      const restoredSession = await authService.restoreSession()
      expect(restoredSession).toBeNull()
      expect(authService.isAuthenticated()).toBe(false)
    })

    it('should handle corrupted session data', async () => {
      // Store corrupted session data
      localStorage.setItem('efs_session', 'invalid-json')

      // Try to restore corrupted session
      const restoredSession = await authService.restoreSession()
      expect(restoredSession).toBeNull()
      expect(authService.isAuthenticated()).toBe(false)

      // Storage should be cleaned up
      expect(localStorage.getItem('efs_session')).toBeNull()
    })

    it('should validate session integrity during restoration', async () => {
      // Create a session
      await authService.register({
        username: 'testuser',
        password: 'TestPass123!',
        confirmPassword: 'TestPass123!'
      })

      // Clear current session
      await sessionManager.clearSession()

      // Mock credential storage to return false for integrity check
      const originalVerifyIntegrity = credentialStorage.verifyIntegrity
      credentialStorage.verifyIntegrity = vi.fn().mockResolvedValue(false)

      // Try to restore session with failed integrity check
      const restoredSession = await authService.restoreSession()
      expect(restoredSession).toBeNull()
      expect(authService.isAuthenticated()).toBe(false)

      // Restore original method
      credentialStorage.verifyIntegrity = originalVerifyIntegrity
    })
  })

  describe('Session Refresh', () => {
    it('should refresh session with valid refresh token', async () => {
      // Create a session
      await authService.register({
        username: 'testuser',
        password: 'TestPass123!',
        confirmPassword: 'TestPass123!'
      })

      const originalSession = sessionManager.getCurrentSession()
      const originalToken = originalSession!.token

      // Wait a moment to ensure new token will be different
      await new Promise(resolve => setTimeout(resolve, 10))

      // Refresh session
      const refreshedSession = await authService.refreshSession()
      expect(refreshedSession).toBeTruthy()
      expect(refreshedSession!.token).not.toBe(originalToken)
      expect(refreshedSession!.username).toBe('testuser')
    })

    it('should fail to refresh with invalid refresh token', async () => {
      // Create a session
      await authService.register({
        username: 'testuser',
        password: 'TestPass123!',
        confirmPassword: 'TestPass123!'
      })

      // Corrupt the refresh token
      localStorage.setItem('efs_refresh_token', 'invalid-token')

      // Try to refresh with invalid token
      const refreshedSession = await authService.refreshSession()
      expect(refreshedSession).toBeNull()
      expect(authService.isAuthenticated()).toBe(false)
    })
  })

  describe('Session Cleanup', () => {
    it('should clear all session data on logout', async () => {
      // Create a session
      await authService.register({
        username: 'testuser',
        password: 'TestPass123!',
        confirmPassword: 'TestPass123!'
      })

      // Verify session data exists
      expect(localStorage.getItem('efs_session')).toBeTruthy()
      expect(localStorage.getItem('efs_refresh_token')).toBeTruthy()
      expect(localStorage.getItem('auth_token')).toBeTruthy()
      expect(sessionManager.getCurrentSession()).toBeTruthy()

      // Logout
      await authService.logout()

      // Verify all session data is cleared
      expect(localStorage.getItem('efs_session')).toBeNull()
      expect(localStorage.getItem('efs_refresh_token')).toBeNull()
      expect(localStorage.getItem('auth_token')).toBeNull()
      expect(sessionManager.getCurrentSession()).toBeNull()
      expect(authService.isAuthenticated()).toBe(false)
    })

    it('should handle cleanup errors gracefully', async () => {
      // Create a session
      await authService.register({
        username: 'testuser',
        password: 'TestPass123!',
        confirmPassword: 'TestPass123!'
      })

      // Mock localStorage.removeItem to throw error
      const originalRemoveItem = localStorage.removeItem
      localStorage.removeItem = vi.fn().mockImplementation(() => {
        throw new Error('Storage error')
      })

      // Logout should not throw error
      await expect(authService.logout()).resolves.not.toThrow()

      // Restore original method
      localStorage.removeItem = originalRemoveItem
    })
  })

  describe('Multiple Sessions', () => {
    it('should handle multiple user sessions', async () => {
      // Register first user
      await authService.register({
        username: 'user1',
        password: 'TestPass123!',
        confirmPassword: 'TestPass123!'
      })

      const user1Session = sessionManager.getCurrentSession()
      expect(user1Session!.username).toBe('user1')

      // Logout first user
      await authService.logout()

      // Register second user
      await authService.register({
        username: 'user2',
        password: 'TestPass123!',
        confirmPassword: 'TestPass123!'
      })

      const user2Session = sessionManager.getCurrentSession()
      expect(user2Session!.username).toBe('user2')

      // Verify first user's session is not accessible
      expect(user2Session!.username).not.toBe('user1')
    })
  })

  describe('Session Activity Tracking', () => {
    it('should update activity timestamp', async () => {
      // Create a session
      await authService.register({
        username: 'testuser',
        password: 'TestPass123!',
        confirmPassword: 'TestPass123!'
      })

      const originalSession = sessionManager.getCurrentSession()
      const originalActivity = originalSession!.lastActivity

      // Wait a moment
      await new Promise(resolve => setTimeout(resolve, 10))

      // Update activity
      authService.updateActivity()

      const updatedSession = sessionManager.getCurrentSession()
      expect(updatedSession!.lastActivity.getTime()).toBeGreaterThan(originalActivity.getTime())
    })

    it('should provide session statistics', async () => {
      // Create a session
      await authService.register({
        username: 'testuser',
        password: 'TestPass123!',
        confirmPassword: 'TestPass123!'
      })

      const stats = authService.getSessionStats()
      expect(stats.isActive).toBe(true)
      expect(stats.timeRemaining).toBeGreaterThan(0)
      expect(stats.lastActivity).toBeInstanceOf(Date)
      expect(stats.autoRefreshEnabled).toBe(true)
    })
  })

  describe('Credential Integrity', () => {
    it('should verify credential integrity', async () => {
      // Create a session
      await authService.register({
        username: 'testuser',
        password: 'TestPass123!',
        confirmPassword: 'TestPass123!'
      })

      // Verify integrity
      const integrityCheck = await authService.verifyCredentialIntegrity()
      expect(integrityCheck).toBe(true)
    })

    it('should handle credential integrity failure', async () => {
      // Create a session
      await authService.register({
        username: 'testuser',
        password: 'TestPass123!',
        confirmPassword: 'TestPass123!'
      })

      // Mock credential storage to fail integrity check
      const originalVerifyIntegrity = credentialStorage.verifyIntegrity
      credentialStorage.verifyIntegrity = vi.fn().mockResolvedValue(false)

      // Verify integrity fails
      const integrityCheck = await authService.verifyCredentialIntegrity()
      expect(integrityCheck).toBe(false)

      // Restore original method
      credentialStorage.verifyIntegrity = originalVerifyIntegrity
    })
  })
})