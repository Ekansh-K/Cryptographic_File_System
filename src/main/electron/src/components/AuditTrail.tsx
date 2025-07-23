import React, { useState, useEffect } from 'react'
import {
  Card,
  Table,
  Tag,
  Typography,
  Space,
  Button,
  DatePicker,
  Select,
  Input,
  Tooltip,
  Modal,
  Descriptions,
  Empty,
  message,
} from 'antd'
import {
  HistoryOutlined,
  SearchOutlined,
  FilterOutlined,
  EyeOutlined,
  DownloadOutlined,
  ReloadOutlined,
} from '@ant-design/icons'
import { motion } from 'framer-motion'
import dayjs from 'dayjs'
import type { ColumnsType } from 'antd/es/table'
import { auditService, AuditEvent, AuditEventType } from '../services/auditService'

const { Title, Text } = Typography
const { RangePicker } = DatePicker
const { Option } = Select
const { Search } = Input

interface AuditTrailProps {
  shareId?: string
  containerId?: string
  title?: string
  height?: number
}

const AuditTrail: React.FC<AuditTrailProps> = ({
  shareId,
  containerId,
  title = 'Audit Trail',
  height = 400,
}) => {
  const [events, setEvents] = useState<AuditEvent[]>([])
  const [loading, setLoading] = useState(false)
  const [selectedEvent, setSelectedEvent] = useState<AuditEvent | null>(null)
  const [detailsVisible, setDetailsVisible] = useState(false)
  const [filters, setFilters] = useState({
    eventType: undefined as AuditEventType | undefined,
    dateRange: undefined as [dayjs.Dayjs, dayjs.Dayjs] | undefined,
    searchText: '',
  })
  const [pagination, setPagination] = useState({
    current: 1,
    pageSize: 20,
    total: 0,
  })

  // Load audit events
  const loadAuditEvents = async (page = 1, pageSize = 20) => {
    setLoading(true)
    try {
      const filter = {
        shareId,
        containerId,
        eventType: filters.eventType,
        startDate: filters.dateRange?.[0]?.toISOString(),
        endDate: filters.dateRange?.[1]?.toISOString(),
        page,
        limit: pageSize,
      }

      const result = await auditService.getAuditLogs(filter)
      
      // Filter by search text locally if provided
      let filteredEvents = result.events
      if (filters.searchText) {
        const searchLower = filters.searchText.toLowerCase()
        filteredEvents = result.events.filter(event =>
          event.containerName.toLowerCase().includes(searchLower) ||
          event.username.toLowerCase().includes(searchLower) ||
          auditService.formatAuditEvent(event).toLowerCase().includes(searchLower)
        )
      }

      setEvents(filteredEvents)
      setPagination({
        current: page,
        pageSize,
        total: result.total,
      })
    } catch (error) {
      console.error('Failed to load audit events:', error)
      message.error('Failed to load audit events')
    } finally {
      setLoading(false)
    }
  }

  // Load events on component mount and filter changes
  useEffect(() => {
    loadAuditEvents()
  }, [shareId, containerId, filters.eventType, filters.dateRange])

  // Handle search
  const handleSearch = (value: string) => {
    setFilters(prev => ({ ...prev, searchText: value }))
    loadAuditEvents(1, pagination.pageSize)
  }

  // Handle event type filter
  const handleEventTypeFilter = (value: AuditEventType | undefined) => {
    setFilters(prev => ({ ...prev, eventType: value }))
  }

  // Handle date range filter
  const handleDateRangeFilter = (dates: [dayjs.Dayjs, dayjs.Dayjs] | null) => {
    setFilters(prev => ({ ...prev, dateRange: dates || undefined }))
  }

  // Show event details
  const showEventDetails = (event: AuditEvent) => {
    setSelectedEvent(event)
    setDetailsVisible(true)
  }

  // Export audit report
  const exportAuditReport = async () => {
    try {
      const startDate = filters.dateRange?.[0]?.toISOString() || dayjs().subtract(30, 'days').toISOString()
      const endDate = filters.dateRange?.[1]?.toISOString() || dayjs().toISOString()
      
      const report = await auditService.generateAuditReport(startDate, endDate)
      
      // Create and download CSV
      const csvContent = [
        'Timestamp,Event Type,User,Container,Details',
        ...report.events.map(event => [
          event.timestamp,
          event.type,
          event.username,
          event.containerName,
          auditService.formatAuditEvent(event).replace(/,/g, ';')
        ].join(','))
      ].join('\n')
      
      const blob = new Blob([csvContent], { type: 'text/csv' })
      const url = URL.createObjectURL(blob)
      const link = document.createElement('a')
      link.href = url
      link.download = `audit-report-${dayjs().format('YYYY-MM-DD')}.csv`
      link.click()
      URL.revokeObjectURL(url)
      
      message.success('Audit report exported successfully')
    } catch (error) {
      console.error('Failed to export audit report:', error)
      message.error('Failed to export audit report')
    }
  }

  // Get event type color
  const getEventTypeColor = (type: AuditEventType): string => {
    switch (type) {
      case AuditEventType.SHARE_CREATED:
        return 'blue'
      case AuditEventType.SHARE_ACCEPTED:
        return 'green'
      case AuditEventType.SHARE_DECLINED:
        return 'orange'
      case AuditEventType.SHARE_REVOKED:
        return 'red'
      case AuditEventType.SHARE_EXPIRED:
        return 'gray'
      case AuditEventType.SHARE_ACCESSED:
        return 'cyan'
      case AuditEventType.PERMISSION_UPDATED:
        return 'purple'
      case AuditEventType.EXPIRATION_EXTENDED:
        return 'lime'
      default:
        return 'default'
    }
  }

  // Get event type label
  const getEventTypeLabel = (type: AuditEventType): string => {
    switch (type) {
      case AuditEventType.SHARE_CREATED:
        return 'Created'
      case AuditEventType.SHARE_ACCEPTED:
        return 'Accepted'
      case AuditEventType.SHARE_DECLINED:
        return 'Declined'
      case AuditEventType.SHARE_REVOKED:
        return 'Revoked'
      case AuditEventType.SHARE_EXPIRED:
        return 'Expired'
      case AuditEventType.SHARE_ACCESSED:
        return 'Accessed'
      case AuditEventType.PERMISSION_UPDATED:
        return 'Permission Updated'
      case AuditEventType.EXPIRATION_EXTENDED:
        return 'Expiration Extended'
      default:
        return type
    }
  }

  // Table columns
  const columns: ColumnsType<AuditEvent> = [
    {
      title: 'Timestamp',
      dataIndex: 'timestamp',
      key: 'timestamp',
      width: 180,
      render: (timestamp: string) => (
        <Tooltip title={dayjs(timestamp).format('YYYY-MM-DD HH:mm:ss')}>
          <Text className="text-sm">{dayjs(timestamp).fromNow()}</Text>
        </Tooltip>
      ),
      sorter: (a, b) => dayjs(a.timestamp).unix() - dayjs(b.timestamp).unix(),
      defaultSortOrder: 'descend',
    },
    {
      title: 'Event',
      dataIndex: 'type',
      key: 'type',
      width: 140,
      render: (type: AuditEventType) => (
        <Tag color={getEventTypeColor(type)}>
          {getEventTypeLabel(type)}
        </Tag>
      ),
      filters: Object.values(AuditEventType).map(type => ({
        text: getEventTypeLabel(type),
        value: type,
      })),
      onFilter: (value, record) => record.type === value,
    },
    {
      title: 'User',
      dataIndex: 'username',
      key: 'username',
      width: 120,
      render: (username: string) => (
        <Text strong className="text-sm">{username}</Text>
      ),
    },
    {
      title: 'Container',
      dataIndex: 'containerName',
      key: 'containerName',
      width: 150,
      render: (containerName: string) => (
        <Text className="text-sm">{containerName}</Text>
      ),
    },
    {
      title: 'Description',
      key: 'description',
      render: (_, record: AuditEvent) => (
        <Text className="text-sm">
          {auditService.formatAuditEvent(record)}
        </Text>
      ),
    },
    {
      title: 'Actions',
      key: 'actions',
      width: 80,
      render: (_, record: AuditEvent) => (
        <Tooltip title="View Details">
          <Button
            type="text"
            size="small"
            icon={<EyeOutlined />}
            onClick={() => showEventDetails(record)}
          />
        </Tooltip>
      ),
    },
  ]

  return (
    <motion.div
      initial={{ opacity: 0, y: 20 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.3 }}
    >
      <Card
        title={
          <div className="flex items-center space-x-2">
            <HistoryOutlined className="text-primary-500" />
            <span>{title}</span>
          </div>
        }
        extra={
          <Space>
            <Button
              icon={<DownloadOutlined />}
              onClick={exportAuditReport}
              size="small"
            >
              Export
            </Button>
            <Button
              icon={<ReloadOutlined />}
              onClick={() => loadAuditEvents(pagination.current, pagination.pageSize)}
              loading={loading}
              size="small"
            />
          </Space>
        }
        className="glass-card"
      >
        {/* Filters */}
        <div className="mb-4 flex flex-wrap items-center gap-4">
          <Search
            placeholder="Search events..."
            allowClear
            style={{ width: 250 }}
            onSearch={handleSearch}
            prefix={<SearchOutlined />}
          />
          
          <Select
            placeholder="Event Type"
            allowClear
            style={{ width: 150 }}
            value={filters.eventType}
            onChange={handleEventTypeFilter}
            suffixIcon={<FilterOutlined />}
          >
            {Object.values(AuditEventType).map(type => (
              <Option key={type} value={type}>
                {getEventTypeLabel(type)}
              </Option>
            ))}
          </Select>
          
          <RangePicker
            placeholder={['Start Date', 'End Date']}
            value={filters.dateRange}
            onChange={handleDateRangeFilter}
            showTime
            format="YYYY-MM-DD HH:mm"
          />
        </div>

        {/* Events Table */}
        <Table
          columns={columns}
          dataSource={events}
          rowKey="id"
          loading={loading}
          pagination={{
            ...pagination,
            showSizeChanger: true,
            showQuickJumper: true,
            showTotal: (total, range) =>
              `${range[0]}-${range[1]} of ${total} events`,
            onChange: (page, pageSize) => {
              loadAuditEvents(page, pageSize)
            },
          }}
          scroll={{ y: height }}
          size="small"
          locale={{
            emptyText: (
              <Empty
                image={Empty.PRESENTED_IMAGE_SIMPLE}
                description="No audit events found"
              />
            ),
          }}
        />
      </Card>

      {/* Event Details Modal */}
      <Modal
        title="Event Details"
        open={detailsVisible}
        onCancel={() => setDetailsVisible(false)}
        footer={null}
        width={600}
      >
        {selectedEvent && (
          <Descriptions column={1} bordered>
            <Descriptions.Item label="Event ID">
              {selectedEvent.id}
            </Descriptions.Item>
            <Descriptions.Item label="Type">
              <Tag color={getEventTypeColor(selectedEvent.type)}>
                {getEventTypeLabel(selectedEvent.type)}
              </Tag>
            </Descriptions.Item>
            <Descriptions.Item label="User">
              {selectedEvent.username}
            </Descriptions.Item>
            <Descriptions.Item label="Container">
              {selectedEvent.containerName}
            </Descriptions.Item>
            <Descriptions.Item label="Timestamp">
              {dayjs(selectedEvent.timestamp).format('YYYY-MM-DD HH:mm:ss')}
            </Descriptions.Item>
            {selectedEvent.ipAddress && (
              <Descriptions.Item label="IP Address">
                {selectedEvent.ipAddress}
              </Descriptions.Item>
            )}
            {selectedEvent.userAgent && (
              <Descriptions.Item label="User Agent">
                {selectedEvent.userAgent}
              </Descriptions.Item>
            )}
            <Descriptions.Item label="Details">
              <pre className="whitespace-pre-wrap text-sm">
                {JSON.stringify(selectedEvent.details, null, 2)}
              </pre>
            </Descriptions.Item>
            <Descriptions.Item label="Description">
              {auditService.formatAuditEvent(selectedEvent)}
            </Descriptions.Item>
          </Descriptions>
        )}
      </Modal>
    </motion.div>
  )
}

export default AuditTrail