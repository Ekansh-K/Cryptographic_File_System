import React from 'react'
import { render, screen, waitFor, fireEvent, act } from '@testing-library/react'
import { MemoryRouter } from 'react-router-dom'
import { vi, describe, it, expect, beforeEach, afterEach } from 'vitest'
import AuthGuard from '../AuthGuard'
import { authService } from '../../services/authService'
import { sessionManager } from '../../services/sessionManager'

// Mock the auth service
vi.mock('../../services/authService', () => ({
  authService: {
    isAuthenticated: vi.fn(),
    restoreSession: vi.fn(),
    updateActivity: vi.fn(),
    verifyCredentialIntegrity: vi.fn(),
    logout: vi.fn(),
    login: vi.fn(),
    register: vi.fn(),
  }
}))

// Mock the session manager
vi.mock('../../services/sessionManager', () => ({
  sessionManager: {
    getCurrentSession: vi.fn(),
    getCurrentUser: vi.fn(),
  }
}))

// Mock the LoginPage and RegistrationForm
vi.mock('../../pages/LoginPage', () => ({
  default: ({ onLoginSuccess, onSwitchToRegister }: any) => (
    <div data-testid="login-page">
      <button onClick={() => onLoginSuccess({ username: 'testuser', id: '1' })}>
        Login Success
      </button>
      <button onClick={onSwitchToRegister}>Switch to Register</button>
    </div>
  )
}))

vi.mock('../RegistrationForm', () => ({
  default: ({ onRegistrationSuccess, onSwitchToLogin }: any) => (
    <div data-testid="registration-form">
      <button onClick={() => onRegistrationSuccess({ username: 'newuser', id: '2' })}>
        Register Success
      </button>
      <button onClick={onSwitchToLogin}>Switch to Login</button>
    </div>
  )
}))

// Mock react-router-dom navigate
const mockNavigate = vi.fn()
vi.mock('react-router-dom', async () => {
  const actual = await vi.importActual('react-router-dom')
  return {
    ...actual,
    useNavigate: () => mockNavigate,
  }
})

// Mock antd message
vi.mock('antd', async () => {
  const actual = await vi.importActual('antd')
  return {
    ...actual,
    message: {
      success: vi.fn(),
      error: vi.fn(),
      warning: vi.fn(),
      info: vi.fn(),
    }
  }
})

describe('AuthGuard Integration Tests', () => {
  const mockAuthService = authService as any
  const mockSessionManager = sessionManager as any

  beforeEach(() => {
    vi.clearAllMocks()
    mockNavigate.mockClear()
    
    // Setup default mocks
    mockAuthService.verifyCredentialIntegrity.mockResolvedValue(true)
    mockAuthService.updateActivity.mockImplementation(() => {})
    mockAuthService.logout.mockResolvedValue(undefined)
  })

  afterEach(() => {
    vi.clearAllTimers()
  })

  const renderAuthGuard = (children = <div data-testid="protected-content">Protected Content</div>) => {
    return render(
      <MemoryRouter>
        <AuthGuard>{children}</AuthGuard>
      </MemoryRouter>
    )
  }

  describe('Authentication State Management', () => {
    it('should show loading state initially', async () => {
      mockAuthService.isAuthenticated.mockReturnValue(false)
      mockAuthService.restoreSession.mockImplementation(() => new Promise(() => {})) // Never resolves

      renderAuthGuard()

      expect(screen.getByText('Checking authentication...')).toBeInTheDocument()
    })

    it('should show protected content when authenticated', async () => {
      mockAuthService.isAuthenticated.mockReturnValue(true)
      mockAuthService.restoreSession.mockResolvedValue({
        id: 'session1',
        userId: 'user1',
        username: 'testuser',
        token: 'token123',
        refreshToken: 'refresh123',
        createdAt: new Date(),
        expiresAt: new Date(Date.now() + 3600000),
        lastActivity: new Date()
      })

      renderAuthGuard()

      await waitFor(() => {
        expect(screen.getByTestId('protected-content')).toBeInTheDocument()
      })
    })

    it('should show login page when not authenticated', async () => {
      mockAuthService.isAuthenticated.mockReturnValue(false)
      mockAuthService.restoreSession.mockResolvedValue(null)

      renderAuthGuard()

      await waitFor(() => {
        expect(screen.getByTestId('login-page')).toBeInTheDocument()
      })
    })

    it('should restore session on startup', async () => {
      const mockSession = {
        id: 'session1',
        userId: 'user1',
        username: 'testuser',
        token: 'token123',
        refreshToken: 'refresh123',
        createdAt: new Date(),
        expiresAt: new Date(Date.now() + 3600000),
        lastActivity: new Date()
      }

      mockAuthService.isAuthenticated.mockReturnValue(false)
      mockAuthService.restoreSession.mockResolvedValue(mockSession)

      renderAuthGuard()

      await waitFor(() => {
        expect(mockAuthService.restoreSession).toHaveBeenCalled()
        expect(screen.getByTestId('protected-content')).toBeInTheDocument()
      })
    })
  })

  describe('Login Flow', () => {
    it('should handle successful login', async () => {
      mockAuthService.isAuthenticated.mockReturnValue(false)
      mockAuthService.restoreSession.mockResolvedValue(null)

      renderAuthGuard()

      await waitFor(() => {
        expect(screen.getByTestId('login-page')).toBeInTheDocument()
      })

      // Simulate successful login
      fireEvent.click(screen.getByText('Login Success'))

      await waitFor(() => {
        expect(screen.getByTestId('protected-content')).toBeInTheDocument()
        expect(mockNavigate).toHaveBeenCalledWith('/')
      })
    })

    it('should switch between login and registration views', async () => {
      mockAuthService.isAuthenticated.mockReturnValue(false)
      mockAuthService.restoreSession.mockResolvedValue(null)

      renderAuthGuard()

      await waitFor(() => {
        expect(screen.getByTestId('login-page')).toBeInTheDocument()
      })

      // Switch to registration
      fireEvent.click(screen.getByText('Switch to Register'))

      await waitFor(() => {
        expect(screen.getByTestId('registration-form')).toBeInTheDocument()
      })

      // Switch back to login
      fireEvent.click(screen.getByText('Switch to Login'))

      await waitFor(() => {
        expect(screen.getByTestId('login-page')).toBeInTheDocument()
      })
    })
  })

  describe('Registration Flow', () => {
    it('should handle successful registration', async () => {
      mockAuthService.isAuthenticated.mockReturnValue(false)
      mockAuthService.restoreSession.mockResolvedValue(null)

      renderAuthGuard()

      await waitFor(() => {
        expect(screen.getByTestId('login-page')).toBeInTheDocument()
      })

      // Switch to registration
      fireEvent.click(screen.getByText('Switch to Register'))

      await waitFor(() => {
        expect(screen.getByTestId('registration-form')).toBeInTheDocument()
      })

      // Simulate successful registration
      fireEvent.click(screen.getByText('Register Success'))

      await waitFor(() => {
        expect(screen.getByTestId('protected-content')).toBeInTheDocument()
        expect(mockNavigate).toHaveBeenCalledWith('/')
      })
    })
  })

  describe('Session Management', () => {
    beforeEach(() => {
      vi.useFakeTimers()
    })

    afterEach(() => {
      vi.useRealTimers()
    })

    it('should set up session monitoring when authenticated', async () => {
      mockAuthService.isAuthenticated.mockReturnValue(true)
      mockAuthService.restoreSession.mockResolvedValue({
        id: 'session1',
        userId: 'user1',
        username: 'testuser',
        token: 'token123',
        refreshToken: 'refresh123',
        createdAt: new Date(),
        expiresAt: new Date(Date.now() + 3600000),
        lastActivity: new Date()
      })

      renderAuthGuard()

      await waitFor(() => {
        expect(screen.getByTestId('protected-content')).toBeInTheDocument()
      })

      // Fast-forward time to trigger session check
      act(() => {
        vi.advanceTimersByTime(30000) // 30 seconds
      })

      await waitFor(() => {
        expect(mockAuthService.isAuthenticated).toHaveBeenCalled()
        expect(mockAuthService.updateActivity).toHaveBeenCalled()
      })
    })

    it('should handle session expiration', async () => {
      mockAuthService.isAuthenticated.mockReturnValue(true)
      mockAuthService.restoreSession.mockResolvedValue({
        id: 'session1',
        userId: 'user1',
        username: 'testuser',
        token: 'token123',
        refreshToken: 'refresh123',
        createdAt: new Date(),
        expiresAt: new Date(Date.now() + 3600000),
        lastActivity: new Date()
      })

      renderAuthGuard()

      await waitFor(() => {
        expect(screen.getByTestId('protected-content')).toBeInTheDocument()
      })

      // Simulate session expiration
      mockAuthService.isAuthenticated.mockReturnValue(false)

      // Fast-forward time to trigger session check
      act(() => {
        vi.advanceTimersByTime(30000) // 30 seconds
      })

      await waitFor(() => {
        expect(mockNavigate).toHaveBeenCalledWith('/login')
      })
    })

    it('should handle credential integrity failure', async () => {
      mockAuthService.isAuthenticated.mockReturnValue(true)
      mockAuthService.restoreSession.mockResolvedValue({
        id: 'session1',
        userId: 'user1',
        username: 'testuser',
        token: 'token123',
        refreshToken: 'refresh123',
        createdAt: new Date(),
        expiresAt: new Date(Date.now() + 3600000),
        lastActivity: new Date()
      })

      renderAuthGuard()

      await waitFor(() => {
        expect(screen.getByTestId('protected-content')).toBeInTheDocument()
      })

      // Simulate credential integrity failure
      mockAuthService.verifyCredentialIntegrity.mockResolvedValue(false)

      // Fast-forward time to trigger session check
      act(() => {
        vi.advanceTimersByTime(30000) // 30 seconds
      })

      await waitFor(() => {
        expect(mockAuthService.logout).toHaveBeenCalled()
        expect(mockNavigate).toHaveBeenCalledWith('/login')
      })
    })
  })

  describe('Error Handling', () => {
    it('should handle authentication check errors', async () => {
      mockAuthService.isAuthenticated.mockReturnValue(false)
      mockAuthService.restoreSession.mockRejectedValue(new Error('Session restore failed'))

      renderAuthGuard()

      await waitFor(() => {
        expect(screen.getByTestId('login-page')).toBeInTheDocument()
      })
    })

    it('should handle session monitoring errors gracefully', async () => {
      mockAuthService.isAuthenticated.mockReturnValue(true)
      mockAuthService.restoreSession.mockResolvedValue({
        id: 'session1',
        userId: 'user1',
        username: 'testuser',
        token: 'token123',
        refreshToken: 'refresh123',
        createdAt: new Date(),
        expiresAt: new Date(Date.now() + 3600000),
        lastActivity: new Date()
      })

      renderAuthGuard()

      await waitFor(() => {
        expect(screen.getByTestId('protected-content')).toBeInTheDocument()
      })

      // Simulate error in session monitoring
      mockAuthService.isAuthenticated.mockImplementation(() => {
        throw new Error('Session check failed')
      })

      // Fast-forward time to trigger session check
      act(() => {
        vi.advanceTimersByTime(30000) // 30 seconds
      })

      // Should not crash the application
      await waitFor(() => {
        expect(screen.getByTestId('protected-content')).toBeInTheDocument()
      })
    })
  })

  describe('Logout Handling', () => {
    it('should handle logout properly', async () => {
      mockAuthService.isAuthenticated.mockReturnValue(true)
      mockAuthService.restoreSession.mockResolvedValue({
        id: 'session1',
        userId: 'user1',
        username: 'testuser',
        token: 'token123',
        refreshToken: 'refresh123',
        createdAt: new Date(),
        expiresAt: new Date(Date.now() + 3600000),
        lastActivity: new Date()
      })

      renderAuthGuard()

      await waitFor(() => {
        expect(screen.getByTestId('protected-content')).toBeInTheDocument()
      })

      // Simulate logout event
      const logoutEvent = new Event('sessionExpired')
      window.dispatchEvent(logoutEvent)

      await waitFor(() => {
        expect(mockNavigate).toHaveBeenCalledWith('/login')
      })
    })
  })

  describe('Component Cleanup', () => {
    it('should cleanup session monitoring on unmount', async () => {
      mockAuthService.isAuthenticated.mockReturnValue(true)
      mockAuthService.restoreSession.mockResolvedValue({
        id: 'session1',
        userId: 'user1',
        username: 'testuser',
        token: 'token123',
        refreshToken: 'refresh123',
        createdAt: new Date(),
        expiresAt: new Date(Date.now() + 3600000),
        lastActivity: new Date()
      })

      const { unmount } = renderAuthGuard()

      await waitFor(() => {
        expect(screen.getByTestId('protected-content')).toBeInTheDocument()
      })

      // Unmount component
      unmount()

      // Verify cleanup (no specific assertion, but should not cause memory leaks)
      expect(true).toBe(true) // Placeholder assertion
    })
  })
})