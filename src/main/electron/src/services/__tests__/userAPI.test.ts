import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import axios from 'axios'
import {
  userAPI,
  AuthErrorType,
  type NewUserData,
  type UserCredentials,
  type User,
  type LoginResponse,
  type RegistrationResponse,
  type UserSearchResult
} from '../api'

// Mock axios
vi.mock('axios', () => {
  const mockAxiosInstance = {
    get: vi.fn(),
    post: vi.fn(),
    patch: vi.fn(),
    delete: vi.fn(),
    interceptors: {
      request: { use: vi.fn() },
      response: { use: vi.fn() }
    }
  }

  return {
    default: {
      create: vi.fn(() => mockAxiosInstance),
      interceptors: {
        request: { use: vi.fn() },
        response: { use: vi.fn() }
      }
    }
  }
})

describe('userAPI', () => {
  let mockAxiosInstance: any

  beforeEach(() => {
    vi.clearAllMocks()
    
    // Get the mocked axios instance
    const mockedAxios = vi.mocked(axios)
    mockAxiosInstance = mockedAxios.create()
    
    // Clear localStorage
    Object.defineProperty(window, 'localStorage', {
      value: {
        getItem: vi.fn(),
        setItem: vi.fn(),
        removeItem: vi.fn(),
        clear: vi.fn(),
      },
      writable: true,
    })
  })

  afterEach(() => {
    vi.restoreAllMocks()
  })

  describe('register', () => {
    const mockUserData: NewUserData = {
      username: 'testuser',
      password: 'TestPass123!',
      confirmPassword: 'TestPass123!',
      email: 'test@example.com'
    }

    const mockUser: User = {
      id: 'user_123',
      username: 'testuser',
      email: 'test@example.com',
      createdAt: '2024-01-01T00:00:00Z',
      lastLogin: '2024-01-01T00:00:00Z',
      isActive: true
    }

    it('should successfully register a new user', async () => {
      const mockResponse = {
        data: {
          user: mockUser,
          token: 'mock_token_123'
        }
      }

      mockAxiosInstance.post.mockResolvedValueOnce(mockResponse)

      const result: RegistrationResponse = await userAPI.register(mockUserData)

      expect(mockAxiosInstance.post).toHaveBeenCalledWith('/auth/register', mockUserData)
      expect(result).toEqual({
        success: true,
        user: mockUser,
        token: 'mock_token_123'
      })
    })

    it('should handle registration failure with username taken error', async () => {
      const mockError = {
        response: {
          status: 409,
          data: {
            message: 'Username already exists'
          }
        }
      }

      mockAxiosInstance.post.mockRejectedValueOnce(mockError)

      const result: RegistrationResponse = await userAPI.register(mockUserData)

      expect(result).toEqual({
        success: false,
        error: 'Username already exists'
      })
    })

    it('should handle registration failure with generic error', async () => {
      const mockError = {
        response: {
          status: 500,
          data: {
            message: 'Internal server error'
          }
        }
      }

      mockAxiosInstance.post.mockRejectedValueOnce(mockError)

      const result: RegistrationResponse = await userAPI.register(mockUserData)

      expect(result).toEqual({
        success: false,
        error: 'Internal server error'
      })
    })

    it('should handle network error during registration', async () => {
      mockAxiosInstance.post.mockRejectedValueOnce(new Error('Network error'))

      const result: RegistrationResponse = await userAPI.register(mockUserData)

      expect(result).toEqual({
        success: false,
        error: 'Registration failed'
      })
    })
  })

  describe('login', () => {
    const mockCredentials: UserCredentials = {
      username: 'testuser',
      password: 'TestPass123!',
      rememberMe: true
    }

    const mockUser: User = {
      id: 'user_123',
      username: 'testuser',
      email: 'test@example.com',
      createdAt: '2024-01-01T00:00:00Z',
      lastLogin: '2024-01-01T00:00:00Z',
      isActive: true
    }

    it('should successfully login user', async () => {
      const mockResponse = {
        data: {
          user: mockUser,
          token: 'mock_token_123',
          refreshToken: 'mock_refresh_token_123',
          expiresAt: '2024-01-01T01:00:00Z'
        }
      }

      mockAxiosInstance.post.mockResolvedValueOnce(mockResponse)

      const result: LoginResponse = await userAPI.login(mockCredentials)

      expect(mockAxiosInstance.post).toHaveBeenCalledWith('/auth/login', mockCredentials)
      expect(result).toEqual({
        success: true,
        user: mockUser,
        token: 'mock_token_123',
        refreshToken: 'mock_refresh_token_123',
        expiresAt: '2024-01-01T01:00:00Z'
      })
    })

    it('should handle login failure with invalid credentials', async () => {
      const mockError = {
        response: {
          status: 401,
          data: {
            message: 'Invalid username or password'
          }
        }
      }

      mockAxiosInstance.post.mockRejectedValueOnce(mockError)

      const result: LoginResponse = await userAPI.login(mockCredentials)

      expect(result).toEqual({
        success: false,
        error: 'Invalid username or password',
        requiresRegistration: false
      })
    })

    it('should handle login failure with user not found', async () => {
      const mockError = {
        response: {
          status: 404,
          data: {
            message: 'User not found'
          }
        }
      }

      mockAxiosInstance.post.mockRejectedValueOnce(mockError)

      const result: LoginResponse = await userAPI.login(mockCredentials)

      expect(result).toEqual({
        success: false,
        error: 'User not found',
        requiresRegistration: true
      })
    })

    it('should handle network error during login', async () => {
      mockAxiosInstance.post.mockRejectedValueOnce(new Error('Network error'))

      const result: LoginResponse = await userAPI.login(mockCredentials)

      expect(result).toEqual({
        success: false,
        error: 'Login failed',
        requiresRegistration: false
      })
    })
  })

  describe('logout', () => {
    it('should successfully logout user', async () => {
      mockAxiosInstance.post.mockResolvedValueOnce({ data: {} })

      const result = await userAPI.logout()

      expect(mockAxiosInstance.post).toHaveBeenCalledWith('/auth/logout')
      expect(result).toEqual({ success: true })
    })

    it('should handle logout failure', async () => {
      const mockError = {
        response: {
          data: {
            message: 'Logout failed'
          }
        }
      }

      mockAxiosInstance.post.mockRejectedValueOnce(mockError)

      const result = await userAPI.logout()

      expect(result).toEqual({
        success: false,
        error: 'Logout failed'
      })
    })

    it('should handle network error during logout', async () => {
      mockAxiosInstance.post.mockRejectedValueOnce(new Error('Network error'))

      const result = await userAPI.logout()

      expect(result).toEqual({
        success: false,
        error: 'Logout failed'
      })
    })
  })

  describe('getCurrentUser', () => {
    const mockUser: User = {
      id: 'user_123',
      username: 'testuser',
      email: 'test@example.com',
      createdAt: '2024-01-01T00:00:00Z',
      lastLogin: '2024-01-01T00:00:00Z',
      isActive: true
    }

    it('should successfully get current user', async () => {
      mockAxiosInstance.get.mockResolvedValueOnce({ data: mockUser })

      const result = await userAPI.getCurrentUser()

      expect(mockAxiosInstance.get).toHaveBeenCalledWith('/auth/me')
      expect(result).toEqual(mockUser)
    })

    it('should handle error when getting current user', async () => {
      const mockError = new Error('Unauthorized')
      mockAxiosInstance.get.mockRejectedValueOnce(mockError)

      await expect(userAPI.getCurrentUser()).rejects.toThrow('Unauthorized')
    })
  })

  describe('refreshToken', () => {
    it('should successfully refresh token', async () => {
      const mockAuthResult = {
        user: {
          id: 'user_123',
          username: 'testuser',
          email: 'test@example.com',
          createdAt: '2024-01-01T00:00:00Z',
          lastLogin: '2024-01-01T00:00:00Z',
          isActive: true
        },
        token: 'new_token_123',
        refreshToken: 'new_refresh_token_123',
        expiresAt: '2024-01-01T02:00:00Z'
      }

      mockAxiosInstance.post.mockResolvedValueOnce({ data: mockAuthResult })

      const result = await userAPI.refreshToken()

      expect(mockAxiosInstance.post).toHaveBeenCalledWith('/auth/refresh')
      expect(result).toEqual(mockAuthResult)
    })
  })

  describe('searchUsers', () => {
    it('should successfully search users', async () => {
      const mockSearchResult: UserSearchResult = {
        users: [
          {
            id: 'user_123',
            username: 'testuser1',
            email: 'test1@example.com',
            createdAt: '2024-01-01T00:00:00Z',
            lastLogin: '2024-01-01T00:00:00Z',
            isActive: true
          },
          {
            id: 'user_456',
            username: 'testuser2',
            email: 'test2@example.com',
            createdAt: '2024-01-01T00:00:00Z',
            lastLogin: '2024-01-01T00:00:00Z',
            isActive: true
          }
        ],
        total: 2,
        page: 1,
        limit: 10
      }

      mockAxiosInstance.get.mockResolvedValueOnce({ data: mockSearchResult })

      const result = await userAPI.searchUsers('test', 10, 1)

      expect(mockAxiosInstance.get).toHaveBeenCalledWith('/users/search', {
        params: { q: 'test', limit: 10, page: 1 }
      })
      expect(result).toEqual(mockSearchResult)
    })

    it('should use default parameters for search', async () => {
      const mockSearchResult: UserSearchResult = {
        users: [],
        total: 0,
        page: 1,
        limit: 10
      }

      mockAxiosInstance.get.mockResolvedValueOnce({ data: mockSearchResult })

      await userAPI.searchUsers('test')

      expect(mockAxiosInstance.get).toHaveBeenCalledWith('/users/search', {
        params: { q: 'test', limit: 10, page: 1 }
      })
    })
  })

  describe('getUserByUsername', () => {
    const mockUser: User = {
      id: 'user_123',
      username: 'testuser',
      email: 'test@example.com',
      createdAt: '2024-01-01T00:00:00Z',
      lastLogin: '2024-01-01T00:00:00Z',
      isActive: true
    }

    it('should successfully get user by username', async () => {
      mockAxiosInstance.get.mockResolvedValueOnce({ data: mockUser })

      const result = await userAPI.getUserByUsername('testuser')

      expect(mockAxiosInstance.get).toHaveBeenCalledWith('/users/username/testuser')
      expect(result).toEqual(mockUser)
    })

    it('should return null when user not found', async () => {
      const mockError = {
        response: {
          status: 404
        }
      }

      mockAxiosInstance.get.mockRejectedValueOnce(mockError)

      const result = await userAPI.getUserByUsername('nonexistent')

      expect(result).toBeNull()
    })

    it('should throw error for non-404 errors', async () => {
      const mockError = {
        response: {
          status: 500,
          data: { message: 'Internal server error' }
        }
      }

      mockAxiosInstance.get.mockRejectedValueOnce(mockError)

      await expect(userAPI.getUserByUsername('testuser')).rejects.toEqual(mockError)
    })
  })

  describe('checkUsernameAvailability', () => {
    it('should check username availability successfully', async () => {
      mockAxiosInstance.get.mockResolvedValueOnce({
        data: { available: true }
      })

      const result = await userAPI.checkUsernameAvailability('newuser')

      expect(mockAxiosInstance.get).toHaveBeenCalledWith('/auth/check-username', {
        params: { username: 'newuser' }
      })
      expect(result).toEqual({ available: true })
    })

    it('should return false for taken username', async () => {
      mockAxiosInstance.get.mockResolvedValueOnce({
        data: { available: false }
      })

      const result = await userAPI.checkUsernameAvailability('takenuser')

      expect(result).toEqual({ available: false })
    })
  })

  describe('changePassword', () => {
    it('should successfully change password', async () => {
      mockAxiosInstance.post.mockResolvedValueOnce({ data: {} })

      const result = await userAPI.changePassword('oldpass', 'newpass')

      expect(mockAxiosInstance.post).toHaveBeenCalledWith('/auth/change-password', {
        currentPassword: 'oldpass',
        newPassword: 'newpass'
      })
      expect(result).toEqual({ success: true })
    })

    it('should handle password change failure', async () => {
      const mockError = {
        response: {
          data: {
            message: 'Current password is incorrect'
          }
        }
      }

      mockAxiosInstance.post.mockRejectedValueOnce(mockError)

      const result = await userAPI.changePassword('wrongpass', 'newpass')

      expect(result).toEqual({
        success: false,
        error: 'Current password is incorrect'
      })
    })
  })

  describe('validateSession', () => {
    const mockUser: User = {
      id: 'user_123',
      username: 'testuser',
      email: 'test@example.com',
      createdAt: '2024-01-01T00:00:00Z',
      lastLogin: '2024-01-01T00:00:00Z',
      isActive: true
    }

    it('should validate session successfully', async () => {
      mockAxiosInstance.get.mockResolvedValueOnce({
        data: { user: mockUser }
      })

      const result = await userAPI.validateSession()

      expect(mockAxiosInstance.get).toHaveBeenCalledWith('/auth/validate')
      expect(result).toEqual({
        valid: true,
        user: mockUser
      })
    })

    it('should handle invalid session', async () => {
      mockAxiosInstance.get.mockRejectedValueOnce(new Error('Unauthorized'))

      const result = await userAPI.validateSession()

      expect(result).toEqual({ valid: false })
    })
  })
})