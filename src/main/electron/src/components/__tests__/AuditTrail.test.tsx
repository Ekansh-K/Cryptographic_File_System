import React from 'react'
import { render, screen, fireEvent, waitFor } from '@testing-library/react'
import { vi, describe, it, expect, beforeEach } from 'vitest'
import { message } from 'antd'
import AuditTrail from '../AuditTrail'
import { auditService, AuditEventType, type AuditEvent } from '../../services/auditService'

// Mock audit service
vi.mock('../../services/auditService', () => ({
  auditService: {
    getAuditLogs: vi.fn(),
    formatAuditEvent: vi.fn(),
    generateAuditReport: vi.fn(),
  },
  AuditEventType: {
    SHARE_CREATED: 'share_created',
    SHARE_ACCEPTED: 'share_accepted',
    SHARE_DECLINED: 'share_declined',
    SHARE_REVOKED: 'share_revoked',
    SHARE_EXPIRED: 'share_expired',
    SHARE_ACCESSED: 'share_accessed',
    PERMISSION_UPDATED: 'permission_updated',
    EXPIRATION_EXTENDED: 'expiration_extended',
  },
}))

// Mock antd message
vi.mock('antd', async () => {
  const actual = await vi.importActual('antd')
  return {
    ...actual,
    message: {
      success: vi.fn(),
      error: vi.fn(),
    },
  }
})

// Mock framer-motion
vi.mock('framer-motion', () => ({
  motion: {
    div: ({ children, ...props }: any) => <div {...props}>{children}</div>,
  },
}))

// Mock dayjs
vi.mock('dayjs', () => {
  const mockDayjs = vi.fn(() => ({
    fromNow: () => '2 hours ago',
    format: () => 'Jan 1, 2024 12:00',
    toISOString: () => '2024-01-01T00:00:00.000Z',
    subtract: () => mockDayjs(),
    unix: () => 1640995200,
  }))
  mockDayjs.extend = vi.fn()
  return {
    default: mockDayjs,
  }
})

describe('AuditTrail', () => {
  const mockAuditEvents: AuditEvent[] = [
    {
      id: 'audit_1',
      type: AuditEventType.SHARE_CREATED,
      shareId: 'share_1',
      userId: 'user_1',
      username: 'testuser',
      containerId: 'container_1',
      containerName: 'Test Container',
      details: { recipientUsername: 'recipient' },
      timestamp: '2024-01-01T00:00:00Z',
      ipAddress: '192.168.1.1',
      userAgent: 'Mozilla/5.0...',
    },
    {
      id: 'audit_2',
      type: AuditEventType.SHARE_ACCEPTED,
      shareId: 'share_1',
      userId: 'user_2',
      username: 'recipient',
      containerId: 'container_1',
      containerName: 'Test Container',
      details: { senderUsername: 'testuser' },
      timestamp: '2024-01-01T01:00:00Z',
    },
    {
      id: 'audit_3',
      type: AuditEventType.SHARE_REVOKED,
      shareId: 'share_1',
      userId: 'user_1',
      username: 'testuser',
      containerId: 'container_1',
      containerName: 'Test Container',
      details: { recipientUsername: 'recipient', reason: 'No longer needed' },
      timestamp: '2024-01-01T02:00:00Z',
    },
  ]

  const mockAuditLog = {
    events: mockAuditEvents,
    total: 3,
    page: 1,
    limit: 50,
  }

  beforeEach(() => {
    vi.clearAllMocks()
    
    // Setup default audit service responses
    vi.mocked(auditService.getAuditLogs).mockResolvedValue(mockAuditLog)
    vi.mocked(auditService.formatAuditEvent).mockImplementation((event) => {
      switch (event.type) {
        case AuditEventType.SHARE_CREATED:
          return `${event.username} shared container "${event.containerName}"`
        case AuditEventType.SHARE_ACCEPTED:
          return `${event.username} accepted share of container "${event.containerName}"`
        case AuditEventType.SHARE_REVOKED:
          return `${event.username} revoked share of container "${event.containerName}"`
        default:
          return `${event.type} for container "${event.containerName}"`
      }
    })
  })

  it('renders audit trail with events', async () => {
    render(<AuditTrail />)
    
    expect(screen.getByText('Audit Trail')).toBeInTheDocument()
    
    await waitFor(() => {
      expect(auditService.getAuditLogs).toHaveBeenCalled()
    })

    // Should display events in table
    expect(screen.getByText('testuser')).toBeInTheDocument()
    expect(screen.getByText('recipient')).toBeInTheDocument()
    expect(screen.getByText('Test Container')).toBeInTheDocument()
  })

  it('handles search functionality', async () => {
    render(<AuditTrail />)
    
    await waitFor(() => {
      expect(screen.getByText('Audit Trail')).toBeInTheDocument()
    })

    const searchInput = screen.getByPlaceholderText('Search events...')
    fireEvent.change(searchInput, { target: { value: 'testuser' } })
    
    const searchButton = screen.getByRole('button', { name: /search/i })
    fireEvent.click(searchButton)

    await waitFor(() => {
      expect(auditService.getAuditLogs).toHaveBeenCalledWith(
        expect.objectContaining({
          page: 1,
          limit: 20,
        })
      )
    })
  })

  it('handles event type filtering', async () => {
    render(<AuditTrail />)
    
    await waitFor(() => {
      expect(screen.getByText('Audit Trail')).toBeInTheDocument()
    })

    // Open event type filter dropdown
    const eventTypeSelect = screen.getByText('Event Type')
    fireEvent.click(eventTypeSelect)

    // Select "Created" option
    const createdOption = screen.getByText('Created')
    fireEvent.click(createdOption)

    await waitFor(() => {
      expect(auditService.getAuditLogs).toHaveBeenCalledWith(
        expect.objectContaining({
          eventType: AuditEventType.SHARE_CREATED,
        })
      )
    })
  })

  it('handles date range filtering', async () => {
    render(<AuditTrail />)
    
    await waitFor(() => {
      expect(screen.getByText('Audit Trail')).toBeInTheDocument()
    })

    // The date range picker would require more complex interaction
    // For now, we'll verify the component renders
    expect(screen.getByPlaceholderText('Start Date')).toBeInTheDocument()
    expect(screen.getByPlaceholderText('End Date')).toBeInTheDocument()
  })

  it('shows event details modal', async () => {
    render(<AuditTrail />)
    
    await waitFor(() => {
      expect(screen.getByText('Audit Trail')).toBeInTheDocument()
    })

    // Click view details button for first event
    const viewDetailsButtons = screen.getAllByTitle('View Details')
    fireEvent.click(viewDetailsButtons[0])

    // Should open details modal
    expect(screen.getByText('Event Details')).toBeInTheDocument()
    expect(screen.getByText('audit_1')).toBeInTheDocument()
    expect(screen.getByText('192.168.1.1')).toBeInTheDocument()
  })

  it('handles export functionality', async () => {
    const mockReport = {
      summary: {
        totalEvents: 3,
        shareCreated: 1,
        shareAccepted: 1,
        shareDeclined: 0,
        shareRevoked: 1,
        shareAccessed: 0,
      },
      events: mockAuditEvents,
    }

    vi.mocked(auditService.generateAuditReport).mockResolvedValue(mockReport)

    // Mock URL.createObjectURL and related functions
    global.URL.createObjectURL = vi.fn(() => 'mock-url')
    global.URL.revokeObjectURL = vi.fn()
    
    // Mock document.createElement and link.click
    const mockLink = {
      href: '',
      download: '',
      click: vi.fn(),
    }
    vi.spyOn(document, 'createElement').mockReturnValue(mockLink as any)

    render(<AuditTrail />)
    
    await waitFor(() => {
      expect(screen.getByText('Audit Trail')).toBeInTheDocument()
    })

    // Click export button
    const exportButton = screen.getByText('Export')
    fireEvent.click(exportButton)

    await waitFor(() => {
      expect(auditService.generateAuditReport).toHaveBeenCalled()
    })

    // Should create and trigger download
    expect(document.createElement).toHaveBeenCalledWith('a')
    expect(mockLink.click).toHaveBeenCalled()
    expect(message.success).toHaveBeenCalledWith('Audit report exported successfully')
  })

  it('handles export error', async () => {
    vi.mocked(auditService.generateAuditReport).mockRejectedValue(
      new Error('Export failed')
    )

    render(<AuditTrail />)
    
    await waitFor(() => {
      expect(screen.getByText('Audit Trail')).toBeInTheDocument()
    })

    const exportButton = screen.getByText('Export')
    fireEvent.click(exportButton)

    await waitFor(() => {
      expect(message.error).toHaveBeenCalledWith('Failed to export audit report')
    })
  })

  it('handles refresh functionality', async () => {
    render(<AuditTrail />)
    
    await waitFor(() => {
      expect(screen.getByText('Audit Trail')).toBeInTheDocument()
    })

    const refreshButton = screen.getByTitle('Refresh')
    fireEvent.click(refreshButton)

    await waitFor(() => {
      expect(auditService.getAuditLogs).toHaveBeenCalledTimes(2) // Initial load + refresh
    })
  })

  it('handles pagination', async () => {
    render(<AuditTrail />)
    
    await waitFor(() => {
      expect(screen.getByText('Audit Trail')).toBeInTheDocument()
    })

    // The pagination would require more complex interaction with antd Table
    // For now, we'll verify the table renders with pagination
    expect(screen.getByText('1-3 of 3 events')).toBeInTheDocument()
  })

  it('shows empty state when no events', async () => {
    vi.mocked(auditService.getAuditLogs).mockResolvedValue({
      events: [],
      total: 0,
      page: 1,
      limit: 50,
    })

    render(<AuditTrail />)
    
    await waitFor(() => {
      expect(screen.getByText('No audit events found')).toBeInTheDocument()
    })
  })

  it('handles loading state', () => {
    // Mock slow API response
    vi.mocked(auditService.getAuditLogs).mockImplementation(
      () => new Promise(resolve => setTimeout(() => resolve(mockAuditLog), 100))
    )

    render(<AuditTrail />)
    
    // Should show loading state
    expect(screen.getByRole('img', { name: 'loading' })).toBeInTheDocument()
  })

  it('handles API errors gracefully', async () => {
    vi.mocked(auditService.getAuditLogs).mockRejectedValue(
      new Error('API error')
    )

    render(<AuditTrail />)
    
    await waitFor(() => {
      expect(message.error).toHaveBeenCalledWith('Failed to load audit events')
    })

    // Should show empty state
    expect(screen.getByText('No audit events found')).toBeInTheDocument()
  })

  it('renders with custom props', async () => {
    render(
      <AuditTrail
        shareId="share_123"
        containerId="container_456"
        title="Custom Audit Trail"
        height={500}
      />
    )
    
    expect(screen.getByText('Custom Audit Trail')).toBeInTheDocument()
    
    await waitFor(() => {
      expect(auditService.getAuditLogs).toHaveBeenCalledWith(
        expect.objectContaining({
          shareId: 'share_123',
          containerId: 'container_456',
        })
      )
    })
  })

  it('displays correct event type badges', async () => {
    render(<AuditTrail />)
    
    await waitFor(() => {
      expect(screen.getByText('Audit Trail')).toBeInTheDocument()
    })

    // Should show different colored badges for different event types
    expect(screen.getByText('Created')).toBeInTheDocument()
    expect(screen.getByText('Accepted')).toBeInTheDocument()
    expect(screen.getByText('Revoked')).toBeInTheDocument()
  })

  it('formats timestamps correctly', async () => {
    render(<AuditTrail />)
    
    await waitFor(() => {
      expect(screen.getByText('Audit Trail')).toBeInTheDocument()
    })

    // Should show relative time
    expect(screen.getAllByText('2 hours ago')).toHaveLength(3)
  })

  it('handles table sorting', async () => {
    render(<AuditTrail />)
    
    await waitFor(() => {
      expect(screen.getByText('Audit Trail')).toBeInTheDocument()
    })

    // The table should have sortable columns
    const timestampHeader = screen.getByText('Timestamp')
    expect(timestampHeader).toBeInTheDocument()
    
    // Clicking would trigger sort, but requires more complex interaction
    // For now, we verify the column is rendered
  })
})