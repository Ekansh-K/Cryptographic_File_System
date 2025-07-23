import React from 'react'
import { render, screen, fireEvent, waitFor } from '@testing-library/react'
import { vi, describe, it, expect, beforeEach } from 'vitest'
import LoginPage from '../LoginPage'
import { authService } from '../../services/authService'

// Mock the auth service
vi.mock('../../services/authService', () => ({
  authService: {
    login: vi.fn(),
    isAuthenticated: vi.fn(),
    getCurrentUser: vi.fn(),
  }
}))

// Mock framer-motion to avoid animation issues in tests
vi.mock('framer-motion', () => ({
  motion: {
    div: ({ children, ...props }: any) => <div {...props}>{children}</div>,
  },
  AnimatePresence: ({ children }: any) => <>{children}</>,
}))

describe('LoginPage', () => {
  const mockOnLoginSuccess = vi.fn()
  const mockOnSwitchToRegister = vi.fn()

  beforeEach(() => {
    vi.clearAllMocks()
  })

  const renderLoginPage = () => {
    return render(
      <LoginPage
        onLoginSuccess={mockOnLoginSuccess}
        onSwitchToRegister={mockOnSwitchToRegister}
      />
    )
  }

  it('renders login form with all required fields', () => {
    renderLoginPage()

    expect(screen.getByText('Welcome Back')).toBeInTheDocument()
    expect(screen.getByText('Sign in to access your encrypted containers')).toBeInTheDocument()
    expect(screen.getByPlaceholderText('Enter your username')).toBeInTheDocument()
    expect(screen.getByPlaceholderText('Enter your password')).toBeInTheDocument()
    expect(screen.getByText('Remember me for 7 days')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: /sign in/i })).toBeInTheDocument()
    expect(screen.getByText('Create an account')).toBeInTheDocument()
  })

  it('validates required fields', async () => {
    renderLoginPage()

    const submitButton = screen.getByRole('button', { name: /sign in/i })
    fireEvent.click(submitButton)

    await waitFor(() => {
      expect(screen.getByText('Please enter your username')).toBeInTheDocument()
      expect(screen.getByText('Please enter your password')).toBeInTheDocument()
    })
  })

  it('validates minimum username length', async () => {
    renderLoginPage()

    const usernameInput = screen.getByPlaceholderText('Enter your username')
    const submitButton = screen.getByRole('button', { name: /sign in/i })

    fireEvent.change(usernameInput, { target: { value: 'ab' } })
    fireEvent.click(submitButton)

    await waitFor(() => {
      expect(screen.getByText('Username must be at least 3 characters')).toBeInTheDocument()
    })
  })

  it('calls authService.login with correct credentials on form submission', async () => {
    const mockLoginResult = {
      success: true,
      user: { id: '1', username: 'testuser' },
      token: 'mock-token'
    }
    
    vi.mocked(authService.login).mockResolvedValue(mockLoginResult)
    
    renderLoginPage()

    const usernameInput = screen.getByPlaceholderText('Enter your username')
    const passwordInput = screen.getByPlaceholderText('Enter your password')
    const rememberMeCheckbox = screen.getByRole('checkbox')
    const submitButton = screen.getByRole('button', { name: /sign in/i })

    fireEvent.change(usernameInput, { target: { value: 'testuser' } })
    fireEvent.change(passwordInput, { target: { value: 'password123' } })
    fireEvent.click(rememberMeCheckbox)
    fireEvent.click(submitButton)

    await waitFor(() => {
      expect(authService.login).toHaveBeenCalledWith({
        username: 'testuser',
        password: 'password123',
        rememberMe: true
      })
    })
  })

  it('calls onLoginSuccess when login is successful', async () => {
    const mockUser = { id: '1', username: 'testuser' }
    const mockLoginResult = {
      success: true,
      user: mockUser,
      token: 'mock-token'
    }
    
    vi.mocked(authService.login).mockResolvedValue(mockLoginResult)
    
    renderLoginPage()

    const usernameInput = screen.getByPlaceholderText('Enter your username')
    const passwordInput = screen.getByPlaceholderText('Enter your password')
    const submitButton = screen.getByRole('button', { name: /sign in/i })

    fireEvent.change(usernameInput, { target: { value: 'testuser' } })
    fireEvent.change(passwordInput, { target: { value: 'password123' } })
    fireEvent.click(submitButton)

    await waitFor(() => {
      expect(mockOnLoginSuccess).toHaveBeenCalledWith(mockUser)
    })
  })

  it('displays error message when login fails', async () => {
    const mockLoginResult = {
      success: false,
      error: 'Invalid credentials'
    }
    
    vi.mocked(authService.login).mockResolvedValue(mockLoginResult)
    
    renderLoginPage()

    const usernameInput = screen.getByPlaceholderText('Enter your username')
    const passwordInput = screen.getByPlaceholderText('Enter your password')
    const submitButton = screen.getByRole('button', { name: /sign in/i })

    fireEvent.change(usernameInput, { target: { value: 'testuser' } })
    fireEvent.change(passwordInput, { target: { value: 'wrongpassword' } })
    fireEvent.click(submitButton)

    await waitFor(() => {
      expect(screen.getByText('Invalid credentials')).toBeInTheDocument()
    })
  })

  it('displays registration prompt when user not found', async () => {
    const mockLoginResult = {
      success: false,
      error: 'User not found',
      requiresRegistration: true
    }
    
    vi.mocked(authService.login).mockResolvedValue(mockLoginResult)
    
    renderLoginPage()

    const usernameInput = screen.getByPlaceholderText('Enter your username')
    const passwordInput = screen.getByPlaceholderText('Enter your password')
    const submitButton = screen.getByRole('button', { name: /sign in/i })

    fireEvent.change(usernameInput, { target: { value: 'newuser' } })
    fireEvent.change(passwordInput, { target: { value: 'password123' } })
    fireEvent.click(submitButton)

    await waitFor(() => {
      expect(screen.getByText('User not found. Please register first.')).toBeInTheDocument()
    })
  })

  it('shows loading state during login', async () => {
    // Create a promise that we can control
    let resolveLogin: (value: any) => void
    const loginPromise = new Promise((resolve) => {
      resolveLogin = resolve
    })
    
    vi.mocked(authService.login).mockReturnValue(loginPromise)
    
    renderLoginPage()

    const usernameInput = screen.getByPlaceholderText('Enter your username')
    const passwordInput = screen.getByPlaceholderText('Enter your password')
    const submitButton = screen.getByRole('button', { name: /sign in/i })

    fireEvent.change(usernameInput, { target: { value: 'testuser' } })
    fireEvent.change(passwordInput, { target: { value: 'password123' } })
    fireEvent.click(submitButton)

    // Check loading state
    await waitFor(() => {
      expect(screen.getByText('Signing In...')).toBeInTheDocument()
      expect(submitButton).toHaveClass('ant-btn-loading')
    })

    // Resolve the promise
    resolveLogin!({ success: true, user: { id: '1', username: 'testuser' } })

    await waitFor(() => {
      expect(screen.queryByText('Signing In...')).not.toBeInTheDocument()
    })
  })

  it('calls onSwitchToRegister when create account link is clicked', () => {
    renderLoginPage()

    const createAccountLink = screen.getByText('Create an account')
    fireEvent.click(createAccountLink)

    expect(mockOnSwitchToRegister).toHaveBeenCalled()
  })

  it('clears error when form values change', async () => {
    const mockLoginResult = {
      success: false,
      error: 'Invalid credentials'
    }
    
    vi.mocked(authService.login).mockResolvedValue(mockLoginResult)
    
    renderLoginPage()

    const usernameInput = screen.getByPlaceholderText('Enter your username')
    const passwordInput = screen.getByPlaceholderText('Enter your password')
    const submitButton = screen.getByRole('button', { name: /sign in/i })

    // First, trigger an error
    fireEvent.change(usernameInput, { target: { value: 'testuser' } })
    fireEvent.change(passwordInput, { target: { value: 'wrongpassword' } })
    fireEvent.click(submitButton)

    await waitFor(() => {
      expect(screen.getByText('Invalid credentials')).toBeInTheDocument()
    })

    // Then change form values to clear error
    fireEvent.change(usernameInput, { target: { value: 'newuser' } })

    await waitFor(() => {
      expect(screen.queryByText('Invalid credentials')).not.toBeInTheDocument()
    })
  })

  it('handles unexpected errors gracefully', async () => {
    vi.mocked(authService.login).mockRejectedValue(new Error('Network error'))
    
    renderLoginPage()

    const usernameInput = screen.getByPlaceholderText('Enter your username')
    const passwordInput = screen.getByPlaceholderText('Enter your password')
    const submitButton = screen.getByRole('button', { name: /sign in/i })

    fireEvent.change(usernameInput, { target: { value: 'testuser' } })
    fireEvent.change(passwordInput, { target: { value: 'password123' } })
    fireEvent.click(submitButton)

    await waitFor(() => {
      expect(screen.getByText('An unexpected error occurred. Please try again.')).toBeInTheDocument()
    })
  })

  it('trims username before submission', async () => {
    const mockLoginResult = {
      success: true,
      user: { id: '1', username: 'testuser' },
      token: 'mock-token'
    }
    
    vi.mocked(authService.login).mockResolvedValue(mockLoginResult)
    
    renderLoginPage()

    const usernameInput = screen.getByPlaceholderText('Enter your username')
    const passwordInput = screen.getByPlaceholderText('Enter your password')
    const submitButton = screen.getByRole('button', { name: /sign in/i })

    fireEvent.change(usernameInput, { target: { value: '  testuser  ' } })
    fireEvent.change(passwordInput, { target: { value: 'password123' } })
    fireEvent.click(submitButton)

    await waitFor(() => {
      expect(authService.login).toHaveBeenCalledWith({
        username: 'testuser',
        password: 'password123',
        rememberMe: false
      })
    })
  })
})