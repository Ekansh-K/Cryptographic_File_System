import React from 'react'
import { render, screen, fireEvent, waitFor } from '@testing-library/react'
import { vi, describe, it, expect, beforeEach } from 'vitest'
import { MemoryRouter } from 'react-router-dom'
import AuthGuard from '../AuthGuard'
import { authService } from '../../services/authService'

// Mock the auth service
vi.mock('../../services/authService', () => ({
  authService: {
    isAuthenticated: vi.fn(),
    restoreSession: vi.fn(),
    updateActivity: vi.fn(),
    verifyCredentialIntegrity: vi.fn(),
    logout: vi.fn(),
  }
}))

// Mock the LoginPage and RegistrationForm components
vi.mock('../../pages/LoginPage', () => ({
  default: ({ onLoginSuccess, onSwitchToRegister }: any) => (
    <div data-testid="login-page">
      <button onClick={() => onLoginSuccess({ id: '1', username: 'testuser' })}>
        Mock Login Success
      </button>
      <button onClick={onSwitchToRegister}>
        Switch to Register
      </button>
    </div>
  )
}))

vi.mock('../RegistrationForm', () => ({
  default: ({ onRegistrationSuccess, onSwitchToLogin }: any) => (
    <div data-testid="registration-form">
      <button onClick={() => onRegistrationSuccess({ id: '1', username: 'testuser' })}>
        Mock Registration Success
      </button>
      <button onClick={onSwitchToLogin}>
        Switch to Login
      </button>
    </div>
  )
}))

// Mock framer-motion to avoid animation issues in tests
vi.mock('framer-motion', () => ({
  motion: {
    div: ({ children, ...props }: any) => <div {...props}>{children}</div>,
  },
  AnimatePresence: ({ children }: any) => <>{children}</>,
}))

describe('AuthGuard', () => {
  const mockChildren = <div data-testid="protected-content">Protected Content</div>

  beforeEach(() => {
    vi.clearAllMocks()
  })

  const renderAuthGuard = () => {
    return render(
      <MemoryRouter>
        <AuthGuard>{mockChildren}</AuthGuard>
      </MemoryRouter>
    )
  }

  it('shows loading state initially', async () => {
    // Mock a pending authentication check
    vi.mocked(authService.isAuthenticated).mockReturnValue(false)
    vi.mocked(authService.restoreSession).mockResolvedValue(null)

    renderAuthGuard()

    // Initially should show loading
    expect(screen.getByText('Checking authentication...')).toBeInTheDocument()
    
    // Wait for the auth check to complete and show login page
    await waitFor(() => {
      expect(screen.getByTestId('login-page')).toBeInTheDocument()
    })
  })

  it('shows protected content when user is already authenticated', async () => {
    vi.mocked(authService.isAuthenticated).mockReturnValue(true)
    vi.mocked(authService.restoreSession).mockResolvedValue(null)

    renderAuthGuard()

    await waitFor(() => {
      expect(screen.getByTestId('protected-content')).toBeInTheDocument()
      expect(screen.queryByTestId('login-page')).not.toBeInTheDocument()
      expect(screen.queryByText('Checking authentication...')).not.toBeInTheDocument()
    })
  })

  it('shows protected content when session is restored successfully', async () => {
    const mockSession = {
      id: 'session-1',
      userId: 'user-1',
      token: 'mock-token',
      createdAt: new Date(),
      expiresAt: new Date(),
      lastActivity: new Date()
    }

    vi.mocked(authService.isAuthenticated).mockReturnValue(false)
    vi.mocked(authService.restoreSession).mockResolvedValue(mockSession)

    renderAuthGuard()

    await waitFor(() => {
      expect(screen.getByTestId('protected-content')).toBeInTheDocument()
      expect(screen.queryByTestId('login-page')).not.toBeInTheDocument()
      expect(screen.queryByText('Checking authentication...')).not.toBeInTheDocument()
    })
  })

  it('shows login page when user is not authenticated and session restore fails', async () => {
    vi.mocked(authService.isAuthenticated).mockReturnValue(false)
    vi.mocked(authService.restoreSession).mockResolvedValue(null)

    renderAuthGuard()

    await waitFor(() => {
      expect(screen.getByTestId('login-page')).toBeInTheDocument()
      expect(screen.queryByTestId('protected-content')).not.toBeInTheDocument()
      expect(screen.queryByText('Checking authentication...')).not.toBeInTheDocument()
    })
  })

  it('shows login page when session restore throws an error', async () => {
    vi.mocked(authService.isAuthenticated).mockReturnValue(false)
    vi.mocked(authService.restoreSession).mockRejectedValue(new Error('Session restore failed'))

    // Mock console.error to avoid error output in tests
    const consoleSpy = vi.spyOn(console, 'error').mockImplementation(() => {})

    renderAuthGuard()

    await waitFor(() => {
      expect(screen.getByTestId('login-page')).toBeInTheDocument()
      expect(screen.queryByTestId('protected-content')).not.toBeInTheDocument()
      expect(screen.queryByText('Checking authentication...')).not.toBeInTheDocument()
    })

    expect(consoleSpy).toHaveBeenCalledWith('Auth check failed:', expect.any(Error))
    consoleSpy.mockRestore()
  })

  it('switches to protected content after successful login', async () => {
    vi.mocked(authService.isAuthenticated).mockReturnValue(false)
    vi.mocked(authService.restoreSession).mockResolvedValue(null)

    renderAuthGuard()

    // Wait for login page to appear
    await waitFor(() => {
      expect(screen.getByTestId('login-page')).toBeInTheDocument()
    })

    // Simulate successful login
    const loginSuccessButton = screen.getByText('Mock Login Success')
    fireEvent.click(loginSuccessButton)

    await waitFor(() => {
      expect(screen.getByTestId('protected-content')).toBeInTheDocument()
      expect(screen.queryByTestId('login-page')).not.toBeInTheDocument()
    })
  })

  it('switches to protected content after successful registration', async () => {
    vi.mocked(authService.isAuthenticated).mockReturnValue(false)
    vi.mocked(authService.restoreSession).mockResolvedValue(null)

    renderAuthGuard()

    // Wait for login page to appear
    await waitFor(() => {
      expect(screen.getByTestId('login-page')).toBeInTheDocument()
    })

    // Switch to registration form
    const switchToRegisterButton = screen.getByText('Switch to Register')
    fireEvent.click(switchToRegisterButton)

    await waitFor(() => {
      expect(screen.getByTestId('registration-form')).toBeInTheDocument()
      expect(screen.queryByTestId('login-page')).not.toBeInTheDocument()
    })

    // Simulate successful registration
    const registrationSuccessButton = screen.getByText('Mock Registration Success')
    fireEvent.click(registrationSuccessButton)

    await waitFor(() => {
      expect(screen.getByTestId('protected-content')).toBeInTheDocument()
      expect(screen.queryByTestId('registration-form')).not.toBeInTheDocument()
    })
  })

  it('switches between login and registration forms', async () => {
    vi.mocked(authService.isAuthenticated).mockReturnValue(false)
    vi.mocked(authService.restoreSession).mockResolvedValue(null)

    renderAuthGuard()

    // Wait for login page to appear
    await waitFor(() => {
      expect(screen.getByTestId('login-page')).toBeInTheDocument()
    })

    // Switch to registration form
    const switchToRegisterButton = screen.getByText('Switch to Register')
    fireEvent.click(switchToRegisterButton)

    await waitFor(() => {
      expect(screen.getByTestId('registration-form')).toBeInTheDocument()
      expect(screen.queryByTestId('login-page')).not.toBeInTheDocument()
    })

    // Switch back to login
    const switchToLoginButton = screen.getByText('Switch to Login')
    fireEvent.click(switchToLoginButton)

    await waitFor(() => {
      expect(screen.getByTestId('login-page')).toBeInTheDocument()
      expect(screen.queryByTestId('registration-form')).not.toBeInTheDocument()
    })
  })

  it('logs successful login to console', async () => {
    const consoleSpy = vi.spyOn(console, 'log').mockImplementation(() => {})
    
    vi.mocked(authService.isAuthenticated).mockReturnValue(false)
    vi.mocked(authService.restoreSession).mockResolvedValue(null)

    renderAuthGuard()

    // Wait for login page to appear
    await waitFor(() => {
      expect(screen.getByTestId('login-page')).toBeInTheDocument()
    })

    // Simulate successful login
    const loginSuccessButton = screen.getByText('Mock Login Success')
    fireEvent.click(loginSuccessButton)

    expect(consoleSpy).toHaveBeenCalledWith('Login successful:', { id: '1', username: 'testuser' })
    
    consoleSpy.mockRestore()
  })

  it('logs successful registration to console', async () => {
    const consoleSpy = vi.spyOn(console, 'log').mockImplementation(() => {})
    
    vi.mocked(authService.isAuthenticated).mockReturnValue(false)
    vi.mocked(authService.restoreSession).mockResolvedValue(null)

    renderAuthGuard()

    // Wait for login page to appear and switch to registration
    await waitFor(() => {
      expect(screen.getByTestId('login-page')).toBeInTheDocument()
    })

    const switchToRegisterButton = screen.getByText('Switch to Register')
    fireEvent.click(switchToRegisterButton)

    await waitFor(() => {
      expect(screen.getByTestId('registration-form')).toBeInTheDocument()
    })

    // Simulate successful registration
    const registrationSuccessButton = screen.getByText('Mock Registration Success')
    fireEvent.click(registrationSuccessButton)

    expect(consoleSpy).toHaveBeenCalledWith('Registration successful:', { id: '1', username: 'testuser' })
    
    consoleSpy.mockRestore()
  })

  it('handles authentication check when isAuthenticated returns true but restoreSession is called', async () => {
    // This tests the case where isAuthenticated returns true, so restoreSession shouldn't be called
    vi.mocked(authService.isAuthenticated).mockReturnValue(true)
    vi.mocked(authService.restoreSession).mockResolvedValue(null)

    renderAuthGuard()

    await waitFor(() => {
      expect(screen.getByTestId('protected-content')).toBeInTheDocument()
    })

    // restoreSession should not have been called since user was already authenticated
    expect(authService.restoreSession).not.toHaveBeenCalled()
  })
})