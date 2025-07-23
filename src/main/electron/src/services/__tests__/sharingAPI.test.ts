import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import axios from 'axios'
import { 
  sharingAPI, 
  SharePermission, 
  ShareStatus,
  SharingErrorType,
  type SharedContainer, 
  type ReceivedShare, 
  type ShareConfig,
  type Container,
  type User,
  type ShareNotification,
  type NotificationPreferences,
  type SharingError
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

describe('sharingAPI', () => {
  let mockAxiosInstance: any

  beforeEach(() => {
    vi.clearAllMocks()
    
    // Get the mocked axios instance
    const mockedAxios = vi.mocked(axios)
    mockAxiosInstance = mockedAxios.create()
  })

  afterEach(() => {
    vi.restoreAllMocks()
  })

  describe('shareWithUser', () => {
    const mockShareConfig: ShareConfig = {
      recipientUsername: 'recipient',
      permissions: [SharePermission.READ, SharePermission.WRITE],
      expiresAt: '2024-12-31T23:59:59Z',
      message: 'Sharing this container with you',
      maxAccess: 10
    }

    const mockSharedContainer: SharedContainer = {
      id: 'share_123',
      containerId: 'container_456',
      containerName: 'My Container',
      recipientUsername: 'recipient',
      permissions: [SharePermission.READ, SharePermission.WRITE],
      createdAt: '2024-01-01T00:00:00Z',
      expiresAt: '2024-12-31T23:59:59Z',
      status: ShareStatus.PENDING,
      accessCount: 0
    }

    it('should successfully share container with user', async () => {
      mockAxiosInstance.post.mockResolvedValueOnce({ data: mockSharedContainer })

      const result = await sharingAPI.shareWithUser('container_456', 'recipient', mockShareConfig)

      expect(mockAxiosInstance.post).toHaveBeenCalledWith('/shares/user', {
        containerId: 'container_456',
        username: 'recipient',
        ...mockShareConfig
      })
      expect(result).toEqual(mockSharedContainer)
    })

    it('should handle sharing error', async () => {
      const mockError = new Error('User not found')
      mockAxiosInstance.post.mockRejectedValueOnce(mockError)

      await expect(sharingAPI.shareWithUser('container_456', 'nonexistent', mockShareConfig))
        .rejects.toThrow('User not found')
    })
  })

  describe('getMyShares', () => {
    const mockSharedContainers: SharedContainer[] = [
      {
        id: 'share_123',
        containerId: 'container_456',
        containerName: 'My Container 1',
        recipientUsername: 'user1',
        permissions: [SharePermission.READ],
        createdAt: '2024-01-01T00:00:00Z',
        status: ShareStatus.ACCEPTED,
        accessCount: 5
      },
      {
        id: 'share_124',
        containerId: 'container_789',
        containerName: 'My Container 2',
        recipientUsername: 'user2',
        permissions: [SharePermission.READ, SharePermission.WRITE],
        createdAt: '2024-01-02T00:00:00Z',
        status: ShareStatus.PENDING,
        accessCount: 0
      }
    ]

    it('should successfully get user\'s shared containers', async () => {
      mockAxiosInstance.get.mockResolvedValueOnce({ data: mockSharedContainers })

      const result = await sharingAPI.getMyShares()

      expect(mockAxiosInstance.get).toHaveBeenCalledWith('/shares/my-shares')
      expect(result).toEqual(mockSharedContainers)
    })

    it('should handle empty shares list', async () => {
      mockAxiosInstance.get.mockResolvedValueOnce({ data: [] })

      const result = await sharingAPI.getMyShares()

      expect(result).toEqual([])
    })
  })

  describe('getReceivedShares', () => {
    const mockReceivedShares: ReceivedShare[] = [
      {
        id: 'share_125',
        containerId: 'container_101',
        containerName: 'Shared Container 1',
        senderUsername: 'sender1',
        permissions: [SharePermission.READ],
        createdAt: '2024-01-01T00:00:00Z',
        status: ShareStatus.PENDING,
        message: 'Check this out!'
      },
      {
        id: 'share_126',
        containerId: 'container_102',
        containerName: 'Shared Container 2',
        senderUsername: 'sender2',
        permissions: [SharePermission.READ, SharePermission.WRITE],
        createdAt: '2024-01-02T00:00:00Z',
        status: ShareStatus.ACCEPTED
      }
    ]

    it('should successfully get received shares', async () => {
      mockAxiosInstance.get.mockResolvedValueOnce({ data: mockReceivedShares })

      const result = await sharingAPI.getReceivedShares()

      expect(mockAxiosInstance.get).toHaveBeenCalledWith('/shares/received')
      expect(result).toEqual(mockReceivedShares)
    })

    it('should handle empty received shares list', async () => {
      mockAxiosInstance.get.mockResolvedValueOnce({ data: [] })

      const result = await sharingAPI.getReceivedShares()

      expect(result).toEqual([])
    })
  })

  describe('acceptShare', () => {
    it('should successfully accept a share', async () => {
      mockAxiosInstance.post.mockResolvedValueOnce({ data: {} })

      await sharingAPI.acceptShare('share_123')

      expect(mockAxiosInstance.post).toHaveBeenCalledWith('/shares/share_123/accept')
    })

    it('should handle accept share error', async () => {
      const mockError = new Error('Share not found')
      mockAxiosInstance.post.mockRejectedValueOnce(mockError)

      await expect(sharingAPI.acceptShare('nonexistent'))
        .rejects.toThrow('Share not found')
    })
  })

  describe('declineShare', () => {
    it('should successfully decline a share', async () => {
      mockAxiosInstance.post.mockResolvedValueOnce({ data: {} })

      await sharingAPI.declineShare('share_123')

      expect(mockAxiosInstance.post).toHaveBeenCalledWith('/shares/share_123/decline')
    })

    it('should handle decline share error', async () => {
      const mockError = new Error('Share not found')
      mockAxiosInstance.post.mockRejectedValueOnce(mockError)

      await expect(sharingAPI.declineShare('nonexistent'))
        .rejects.toThrow('Share not found')
    })
  })

  describe('revokeUserShare', () => {
    it('should successfully revoke a share', async () => {
      mockAxiosInstance.delete.mockResolvedValueOnce({ data: {} })

      await sharingAPI.revokeUserShare('share_123')

      expect(mockAxiosInstance.delete).toHaveBeenCalledWith('/shares/share_123')
    })

    it('should handle revoke share error', async () => {
      const mockError = new Error('Share not found')
      mockAxiosInstance.delete.mockRejectedValueOnce(mockError)

      await expect(sharingAPI.revokeUserShare('nonexistent'))
        .rejects.toThrow('Share not found')
    })
  })

  describe('getShareDetails', () => {
    const mockShareDetails: SharedContainer = {
      id: 'share_123',
      containerId: 'container_456',
      containerName: 'My Container',
      recipientUsername: 'recipient',
      permissions: [SharePermission.READ, SharePermission.WRITE],
      createdAt: '2024-01-01T00:00:00Z',
      expiresAt: '2024-12-31T23:59:59Z',
      status: ShareStatus.ACCEPTED,
      accessCount: 3,
      lastAccessed: '2024-01-15T10:30:00Z'
    }

    it('should successfully get share details', async () => {
      mockAxiosInstance.get.mockResolvedValueOnce({ data: mockShareDetails })

      const result = await sharingAPI.getShareDetails('share_123')

      expect(mockAxiosInstance.get).toHaveBeenCalledWith('/shares/share_123')
      expect(result).toEqual(mockShareDetails)
    })

    it('should handle share not found error', async () => {
      const mockError = new Error('Share not found')
      mockAxiosInstance.get.mockRejectedValueOnce(mockError)

      await expect(sharingAPI.getShareDetails('nonexistent'))
        .rejects.toThrow('Share not found')
    })
  })

  describe('updateSharePermissions', () => {
    it('should successfully update share permissions', async () => {
      const newPermissions = [SharePermission.READ]
      mockAxiosInstance.patch.mockResolvedValueOnce({ data: {} })

      await sharingAPI.updateSharePermissions('share_123', newPermissions)

      expect(mockAxiosInstance.patch).toHaveBeenCalledWith('/shares/share_123/permissions', {
        permissions: newPermissions
      })
    })

    it('should handle update permissions error', async () => {
      const mockError = new Error('Insufficient permissions')
      mockAxiosInstance.patch.mockRejectedValueOnce(mockError)

      await expect(sharingAPI.updateSharePermissions('share_123', [SharePermission.READ]))
        .rejects.toThrow('Insufficient permissions')
    })
  })

  describe('extendShareExpiration', () => {
    it('should successfully extend share expiration', async () => {
      const newExpiration = '2025-12-31T23:59:59Z'
      mockAxiosInstance.patch.mockResolvedValueOnce({ data: {} })

      await sharingAPI.extendShareExpiration('share_123', newExpiration)

      expect(mockAxiosInstance.patch).toHaveBeenCalledWith('/shares/share_123/expiration', {
        expiresAt: newExpiration
      })
    })

    it('should handle extend expiration error', async () => {
      const mockError = new Error('Invalid expiration date')
      mockAxiosInstance.patch.mockRejectedValueOnce(mockError)

      await expect(sharingAPI.extendShareExpiration('share_123', 'invalid-date'))
        .rejects.toThrow('Invalid expiration date')
    })
  })

  describe('getSharingStats', () => {
    const mockStats = {
      totalShared: 5,
      totalReceived: 3,
      activeShares: 4,
      pendingShares: 2
    }

    it('should successfully get sharing statistics', async () => {
      mockAxiosInstance.get.mockResolvedValueOnce({ data: mockStats })

      const result = await sharingAPI.getSharingStats()

      expect(mockAxiosInstance.get).toHaveBeenCalledWith('/shares/stats')
      expect(result).toEqual(mockStats)
    })

    it('should handle stats error', async () => {
      const mockError = new Error('Failed to get stats')
      mockAxiosInstance.get.mockRejectedValueOnce(mockError)

      await expect(sharingAPI.getSharingStats())
        .rejects.toThrow('Failed to get stats')
    })
  })

  describe('getShareableContainers', () => {
    const mockContainers: Container[] = [
      {
        id: 'container_1',
        name: 'Container 1',
        path: '/path/to/container1',
        size: 1024,
        status: 'mounted',
        createdAt: '2024-01-01T00:00:00Z',
        lastAccessed: '2024-01-15T10:30:00Z',
        encrypted: true,
        steganographic: false
      },
      {
        id: 'container_2',
        name: 'Container 2',
        path: '/path/to/container2',
        size: 2048,
        status: 'unmounted',
        createdAt: '2024-01-02T00:00:00Z',
        lastAccessed: '2024-01-14T09:15:00Z',
        encrypted: true,
        steganographic: true
      }
    ]

    it('should successfully get shareable containers', async () => {
      mockAxiosInstance.get.mockResolvedValueOnce({ data: mockContainers })

      const result = await sharingAPI.getShareableContainers()

      expect(mockAxiosInstance.get).toHaveBeenCalledWith('/shares/containers')
      expect(result).toEqual(mockContainers)
    })

    it('should handle empty containers list', async () => {
      mockAxiosInstance.get.mockResolvedValueOnce({ data: [] })

      const result = await sharingAPI.getShareableContainers()

      expect(result).toEqual([])
    })

    it('should handle get containers error', async () => {
      const mockError = new Error('Failed to get containers')
      mockAxiosInstance.get.mockRejectedValueOnce(mockError)

      await expect(sharingAPI.getShareableContainers())
        .rejects.toThrow('Failed to get containers')
    })
  })

  // Test new enhanced sharing functionality
  describe('enhanced user search and validation', () => {
    const mockUsers: User[] = [
      {
        id: 'user_1',
        username: 'testuser1',
        email: 'test1@example.com',
        createdAt: '2024-01-01T00:00:00Z',
        lastLogin: '2024-01-01T00:00:00Z',
        isActive: true
      },
      {
        id: 'user_2',
        username: 'testuser2',
        email: 'test2@example.com',
        createdAt: '2024-01-01T00:00:00Z',
        lastLogin: '2024-01-01T00:00:00Z',
        isActive: true
      }
    ]

    describe('searchUsersForSharing', () => {
      it('should successfully search users for sharing', async () => {
        mockAxiosInstance.get.mockResolvedValueOnce({ data: { users: mockUsers } })

        const result = await sharingAPI.searchUsersForSharing('test', 5)

        expect(mockAxiosInstance.get).toHaveBeenCalledWith('/users/search/sharing', {
          params: { q: 'test', limit: 5 }
        })
        expect(result).toEqual(mockUsers)
      })

      it('should use default limit when not specified', async () => {
        mockAxiosInstance.get.mockResolvedValueOnce({ data: { users: mockUsers } })

        await sharingAPI.searchUsersForSharing('test')

        expect(mockAxiosInstance.get).toHaveBeenCalledWith('/users/search/sharing', {
          params: { q: 'test', limit: 5 }
        })
      })

      it('should handle empty search results', async () => {
        mockAxiosInstance.get.mockResolvedValueOnce({ data: {} })

        const result = await sharingAPI.searchUsersForSharing('nonexistent')

        expect(result).toEqual([])
      })

      it('should handle search error', async () => {
        const mockError = new Error('Search failed')
        mockAxiosInstance.get.mockRejectedValueOnce(mockError)

        await expect(sharingAPI.searchUsersForSharing('test'))
          .rejects.toThrow('Search failed')
      })
    })

    describe('validateShareRecipient', () => {
      it('should successfully validate share recipient', async () => {
        const mockValidation = {
          valid: true
        }

        mockAxiosInstance.post.mockResolvedValueOnce({ data: mockValidation })

        const result = await sharingAPI.validateShareRecipient('testuser', 'container_123')

        expect(mockAxiosInstance.post).toHaveBeenCalledWith('/shares/validate-recipient', {
          username: 'testuser',
          containerId: 'container_123'
        })
        expect(result).toEqual(mockValidation)
      })

      it('should handle invalid recipient', async () => {
        const mockValidation = {
          valid: false,
          reason: 'User not found',
          suggestions: ['testuser1', 'testuser2']
        }

        mockAxiosInstance.post.mockResolvedValueOnce({ data: mockValidation })

        const result = await sharingAPI.validateShareRecipient('nonexistent', 'container_123')

        expect(result).toEqual(mockValidation)
      })

      it('should handle validation error', async () => {
        const mockError = {
          response: {
            data: {
              message: 'Container not accessible'
            }
          }
        }

        mockAxiosInstance.post.mockRejectedValueOnce(mockError)

        const result = await sharingAPI.validateShareRecipient('testuser', 'container_123')

        expect(result).toEqual({
          valid: false,
          reason: 'Container not accessible'
        })
      })
    })
  })

  describe('notification management', () => {
    const mockNotifications: ShareNotification[] = [
      {
        id: 'notif_1',
        type: 'share_received',
        shareId: 'share_123',
        fromUsername: 'sender1',
        containerName: 'Shared Container',
        message: 'Check this out!',
        createdAt: '2024-01-01T00:00:00Z',
        read: false
      },
      {
        id: 'notif_2',
        type: 'share_accepted',
        shareId: 'share_124',
        toUsername: 'recipient1',
        containerName: 'My Container',
        createdAt: '2024-01-02T00:00:00Z',
        read: true
      }
    ]

    describe('getNotifications', () => {
      it('should successfully get all notifications', async () => {
        mockAxiosInstance.get.mockResolvedValueOnce({ data: mockNotifications })

        const result = await sharingAPI.getNotifications()

        expect(mockAxiosInstance.get).toHaveBeenCalledWith('/shares/notifications', {
          params: { unreadOnly: false }
        })
        expect(result).toEqual(mockNotifications)
      })

      it('should get only unread notifications', async () => {
        const unreadNotifications = mockNotifications.filter(n => !n.read)
        mockAxiosInstance.get.mockResolvedValueOnce({ data: unreadNotifications })

        const result = await sharingAPI.getNotifications(true)

        expect(mockAxiosInstance.get).toHaveBeenCalledWith('/shares/notifications', {
          params: { unreadOnly: true }
        })
        expect(result).toEqual(unreadNotifications)
      })

      it('should handle empty notifications', async () => {
        mockAxiosInstance.get.mockResolvedValueOnce({ data: [] })

        const result = await sharingAPI.getNotifications()

        expect(result).toEqual([])
      })
    })

    describe('markNotificationRead', () => {
      it('should successfully mark notification as read', async () => {
        mockAxiosInstance.patch.mockResolvedValueOnce({ data: {} })

        await sharingAPI.markNotificationRead('notif_1')

        expect(mockAxiosInstance.patch).toHaveBeenCalledWith('/shares/notifications/notif_1/read')
      })

      it('should handle mark read error', async () => {
        const mockError = new Error('Notification not found')
        mockAxiosInstance.patch.mockRejectedValueOnce(mockError)

        await expect(sharingAPI.markNotificationRead('nonexistent'))
          .rejects.toThrow('Notification not found')
      })
    })

    describe('markAllNotificationsRead', () => {
      it('should successfully mark all notifications as read', async () => {
        mockAxiosInstance.patch.mockResolvedValueOnce({ data: {} })

        await sharingAPI.markAllNotificationsRead()

        expect(mockAxiosInstance.patch).toHaveBeenCalledWith('/shares/notifications/read-all')
      })
    })

    describe('notification preferences', () => {
      const mockPreferences: NotificationPreferences = {
        shareReceived: true,
        shareAccepted: true,
        shareDeclined: false,
        shareRevoked: true,
        shareExpired: true,
        emailNotifications: false,
        pushNotifications: true
      }

      it('should get notification preferences', async () => {
        mockAxiosInstance.get.mockResolvedValueOnce({ data: mockPreferences })

        const result = await sharingAPI.getNotificationPreferences()

        expect(mockAxiosInstance.get).toHaveBeenCalledWith('/shares/notifications/preferences')
        expect(result).toEqual(mockPreferences)
      })

      it('should update notification preferences', async () => {
        const updates = { emailNotifications: true, shareDeclined: true }
        mockAxiosInstance.patch.mockResolvedValueOnce({ data: {} })

        await sharingAPI.updateNotificationPreferences(updates)

        expect(mockAxiosInstance.patch).toHaveBeenCalledWith('/shares/notifications/preferences', updates)
      })
    })
  })

  describe('real-time status updates', () => {
    describe('getShareStatusUpdates', () => {
      it('should successfully get share status updates', async () => {
        const mockUpdates = {
          'share_123': {
            status: ShareStatus.ACCEPTED,
            lastActivity: '2024-01-15T10:30:00Z',
            accessCount: 5
          },
          'share_124': {
            status: ShareStatus.PENDING,
            lastActivity: '2024-01-14T09:15:00Z',
            accessCount: 0
          }
        }

        mockAxiosInstance.post.mockResolvedValueOnce({ data: mockUpdates })

        const result = await sharingAPI.getShareStatusUpdates(['share_123', 'share_124'])

        expect(mockAxiosInstance.post).toHaveBeenCalledWith('/shares/status-updates', {
          shareIds: ['share_123', 'share_124']
        })
        expect(result).toEqual(mockUpdates)
      })

      it('should handle empty share IDs', async () => {
        mockAxiosInstance.post.mockResolvedValueOnce({ data: {} })

        const result = await sharingAPI.getShareStatusUpdates([])

        expect(result).toEqual({})
      })
    })
  })

  describe('bulk operations', () => {
    const mockBulkResult = {
      successful: ['share_123', 'share_124'],
      failed: [
        { shareId: 'share_125', error: 'Share not found' }
      ]
    }

    describe('bulkAcceptShares', () => {
      it('should successfully accept multiple shares', async () => {
        mockAxiosInstance.post.mockResolvedValueOnce({ data: mockBulkResult })

        const result = await sharingAPI.bulkAcceptShares(['share_123', 'share_124', 'share_125'])

        expect(mockAxiosInstance.post).toHaveBeenCalledWith('/shares/bulk-accept', {
          shareIds: ['share_123', 'share_124', 'share_125']
        })
        expect(result).toEqual(mockBulkResult)
      })

      it('should handle bulk accept error', async () => {
        const mockError = new Error('Bulk operation failed')
        mockAxiosInstance.post.mockRejectedValueOnce(mockError)

        await expect(sharingAPI.bulkAcceptShares(['share_123']))
          .rejects.toThrow('Bulk operation failed')
      })
    })

    describe('bulkDeclineShares', () => {
      it('should successfully decline multiple shares', async () => {
        mockAxiosInstance.post.mockResolvedValueOnce({ data: mockBulkResult })

        const result = await sharingAPI.bulkDeclineShares(['share_123', 'share_124', 'share_125'])

        expect(mockAxiosInstance.post).toHaveBeenCalledWith('/shares/bulk-decline', {
          shareIds: ['share_123', 'share_124', 'share_125']
        })
        expect(result).toEqual(mockBulkResult)
      })
    })

    describe('bulkRevokeShares', () => {
      it('should successfully revoke multiple shares', async () => {
        mockAxiosInstance.post.mockResolvedValueOnce({ data: mockBulkResult })

        const result = await sharingAPI.bulkRevokeShares(['share_123', 'share_124', 'share_125'])

        expect(mockAxiosInstance.post).toHaveBeenCalledWith('/shares/bulk-revoke', {
          shareIds: ['share_123', 'share_124', 'share_125']
        })
        expect(result).toEqual(mockBulkResult)
      })
    })
  })

  describe('error handling', () => {
    describe('handleSharingError', () => {
      it('should handle user not found error (404)', () => {
        const mockError = {
          response: {
            status: 404,
            data: {
              message: 'User not found',
              details: { username: 'nonexistent' }
            }
          }
        }

        const result = sharingAPI.handleSharingError(mockError)

        expect(result).toEqual({
          type: SharingErrorType.USER_NOT_FOUND,
          message: 'User not found',
          details: { username: 'nonexistent' },
          retryable: false
        })
      })

      it('should handle container not found error (404)', () => {
        const mockError = {
          response: {
            status: 404,
            data: {
              message: 'Container not found'
            }
          }
        }

        const result = sharingAPI.handleSharingError(mockError)

        expect(result).toEqual({
          type: SharingErrorType.CONTAINER_NOT_FOUND,
          message: 'Container not found',
          details: undefined,
          retryable: false
        })
      })

      it('should handle insufficient permissions error (403)', () => {
        const mockError = {
          response: {
            status: 403,
            data: {
              message: 'Insufficient permissions'
            }
          }
        }

        const result = sharingAPI.handleSharingError(mockError)

        expect(result).toEqual({
          type: SharingErrorType.INSUFFICIENT_PERMISSIONS,
          message: 'Insufficient permissions',
          details: undefined,
          retryable: false
        })
      })

      it('should handle share already exists error (409)', () => {
        const mockError = {
          response: {
            status: 409,
            data: {
              message: 'Share already exists'
            }
          }
        }

        const result = sharingAPI.handleSharingError(mockError)

        expect(result).toEqual({
          type: SharingErrorType.SHARE_ALREADY_EXISTS,
          message: 'Share already exists',
          details: undefined,
          retryable: false
        })
      })

      it('should handle share limit exceeded error (409)', () => {
        const mockError = {
          response: {
            status: 409,
            data: {
              message: 'Share limit exceeded'
            }
          }
        }

        const result = sharingAPI.handleSharingError(mockError)

        expect(result).toEqual({
          type: SharingErrorType.SHARE_LIMIT_EXCEEDED,
          message: 'Share limit exceeded',
          details: undefined,
          retryable: false
        })
      })

      it('should handle share expired error (410)', () => {
        const mockError = {
          response: {
            status: 410,
            data: {
              message: 'Share has expired'
            }
          }
        }

        const result = sharingAPI.handleSharingError(mockError)

        expect(result).toEqual({
          type: SharingErrorType.SHARE_EXPIRED,
          message: 'Share has expired',
          details: undefined,
          retryable: false
        })
      })

      it('should handle container not accessible error (423)', () => {
        const mockError = {
          response: {
            status: 423,
            data: {
              message: 'Container is locked'
            }
          }
        }

        const result = sharingAPI.handleSharingError(mockError)

        expect(result).toEqual({
          type: SharingErrorType.CONTAINER_NOT_ACCESSIBLE,
          message: 'Container is locked',
          details: undefined,
          retryable: false
        })
      })

      it('should handle sharing disabled error (503)', () => {
        const mockError = {
          response: {
            status: 503,
            data: {
              message: 'Sharing service temporarily unavailable'
            }
          }
        }

        const result = sharingAPI.handleSharingError(mockError)

        expect(result).toEqual({
          type: SharingErrorType.SHARING_DISABLED,
          message: 'Sharing service temporarily unavailable',
          details: undefined,
          retryable: true
        })
      })

      it('should handle unknown error with default fallback', () => {
        const mockError = {
          response: {
            status: 500,
            data: {
              message: 'Internal server error'
            }
          }
        }

        const result = sharingAPI.handleSharingError(mockError)

        expect(result).toEqual({
          type: SharingErrorType.INSUFFICIENT_PERMISSIONS,
          message: 'Internal server error',
          details: undefined,
          retryable: true
        })
      })

      it('should handle error without response', () => {
        const mockError = new Error('Network error')

        const result = sharingAPI.handleSharingError(mockError)

        expect(result).toEqual({
          type: SharingErrorType.INSUFFICIENT_PERMISSIONS,
          message: 'Sharing operation failed',
          details: undefined,
          retryable: false
        })
      })
    })
  })

  // Test legacy methods for backward compatibility
  describe('legacy methods', () => {
    it('should support legacy getShares method', async () => {
      const mockShares = [
        {
          id: 'legacy_share_1',
          name: 'Legacy Share',
          files: ['file1.txt', 'file2.txt'],
          createdAt: '2024-01-01T00:00:00Z',
          accessCount: 2
        }
      ]

      mockAxiosInstance.get.mockResolvedValueOnce({ data: mockShares })

      const result = await sharingAPI.getShares()

      expect(mockAxiosInstance.get).toHaveBeenCalledWith('/shares')
      expect(result).toEqual(mockShares)
    })

    it('should support legacy createShare method', async () => {
      const mockShare = {
        id: 'legacy_share_1',
        name: 'Legacy Share',
        files: ['file1.txt', 'file2.txt'],
        createdAt: '2024-01-01T00:00:00Z',
        accessCount: 0
      }

      mockAxiosInstance.post.mockResolvedValueOnce({ data: mockShare })

      const result = await sharingAPI.createShare('container_1', ['file1.txt', 'file2.txt'], {})

      expect(mockAxiosInstance.post).toHaveBeenCalledWith('/shares', {
        containerId: 'container_1',
        files: ['file1.txt', 'file2.txt']
      })
      expect(result).toEqual(mockShare)
    })

    it('should support legacy revokeShare method', async () => {
      mockAxiosInstance.delete.mockResolvedValueOnce({ data: {} })

      await sharingAPI.revokeShare('legacy_share_1')

      expect(mockAxiosInstance.delete).toHaveBeenCalledWith('/shares/legacy_share_1')
    })
  })
})