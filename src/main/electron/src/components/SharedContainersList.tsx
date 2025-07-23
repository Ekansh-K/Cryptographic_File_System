import React, { useState, useEffect } from 'react'
import {
  Card,
  List,
  Button,
  Tag,
  Space,
  Typography,
  Popconfirm,
  message,
  Empty,
  Tooltip,
  Badge,
  Dropdown,
  Modal,
  Descriptions,
  Spin,
  Input,
} from 'antd'
import {
  ShareAltOutlined,
  UserOutlined,
  CalendarOutlined,
  EyeOutlined,
  EditOutlined,
  DeleteOutlined,
  CheckOutlined,
  CloseOutlined,
  MoreOutlined,
  SearchOutlined,
  FilterOutlined,
  ReloadOutlined,
} from '@ant-design/icons'
import { motion, AnimatePresence } from 'framer-motion'
import dayjs from 'dayjs'
import relativeTime from 'dayjs/plugin/relativeTime'
import {
  SharedContainer,
  ReceivedShare,
  SharePermission,
  ShareStatus,
  sharingAPI,
} from '../services/api'
import { auditService, AuditEventType } from '../services/auditService'
import { Badge as CustomBadge } from './ui/badge'

dayjs.extend(relativeTime)

const { Title, Text } = Typography
const { Search } = Input

interface SharedContainersListProps {
  sharedContainers: SharedContainer[]
  receivedShares: ReceivedShare[]
  onRevokeShare: (shareId: string) => Promise<void>
  onAcceptShare: (shareId: string) => Promise<void>
  onDeclineShare: (shareId: string) => Promise<void>
  onRefresh: () => void
  loading?: boolean
}

const SharedContainersList: React.FC<SharedContainersListProps> = ({
  sharedContainers,
  receivedShares,
  onRevokeShare,
  onAcceptShare,
  onDeclineShare,
  onRefresh,
  loading = false,
}) => {
  const [activeTab, setActiveTab] = useState<'shared' | 'received'>('shared')
  const [searchText, setSearchText] = useState('')
  const [statusFilter, setStatusFilter] = useState<ShareStatus | 'all'>('all')
  const [selectedShare, setSelectedShare] = useState<SharedContainer | ReceivedShare | null>(null)
  const [detailsVisible, setDetailsVisible] = useState(false)
  const [actionLoading, setActionLoading] = useState<string | null>(null)

  // Filter shared containers
  const filteredSharedContainers = sharedContainers.filter(share => {
    const matchesSearch = share.containerName.toLowerCase().includes(searchText.toLowerCase()) ||
                         share.recipientUsername.toLowerCase().includes(searchText.toLowerCase())
    const matchesStatus = statusFilter === 'all' || share.status === statusFilter
    return matchesSearch && matchesStatus
  })

  // Filter received shares
  const filteredReceivedShares = receivedShares.filter(share => {
    const matchesSearch = share.containerName.toLowerCase().includes(searchText.toLowerCase()) ||
                         share.senderUsername.toLowerCase().includes(searchText.toLowerCase())
    const matchesStatus = statusFilter === 'all' || share.status === statusFilter
    return matchesSearch && matchesStatus
  })

  // Get status badge variant
  const getStatusBadge = (status: ShareStatus) => {
    switch (status) {
      case ShareStatus.PENDING:
        return <CustomBadge variant="warning">Pending</CustomBadge>
      case ShareStatus.ACCEPTED:
        return <CustomBadge variant="success">Active</CustomBadge>
      case ShareStatus.DECLINED:
        return <CustomBadge variant="secondary">Declined</CustomBadge>
      case ShareStatus.REVOKED:
        return <CustomBadge variant="error">Revoked</CustomBadge>
      case ShareStatus.EXPIRED:
        return <CustomBadge variant="secondary">Expired</CustomBadge>
      default:
        return <CustomBadge variant="default">{status}</CustomBadge>
    }
  }

  // Get permission tags
  const getPermissionTags = (permissions: SharePermission[]) => {
    const permissionConfig = {
      [SharePermission.READ]: { color: 'blue', label: 'Read' },
      [SharePermission.WRITE]: { color: 'orange', label: 'Write' },
      [SharePermission.SHARE]: { color: 'purple', label: 'Share' },
    }

    return permissions.map(permission => (
      <Tag key={permission} color={permissionConfig[permission].color}>
        {permissionConfig[permission].label}
      </Tag>
    ))
  }

  // Handle share action with loading state and audit logging
  const handleShareAction = async (
    action: () => Promise<void>,
    shareId: string,
    successMessage: string,
    auditEventType?: AuditEventType,
    share?: SharedContainer | ReceivedShare
  ) => {
    setActionLoading(shareId)
    try {
      await action()
      
      // Log audit event if specified
      if (auditEventType && share) {
        await auditService.logEvent(
          auditEventType,
          shareId,
          share.containerId,
          {
            containerName: share.containerName,
            permissions: share.permissions,
            ...(activeTab === 'shared' && 'recipientUsername' in share 
              ? { recipientUsername: share.recipientUsername }
              : {}),
            ...(activeTab === 'received' && 'senderUsername' in share 
              ? { senderUsername: share.senderUsername }
              : {})
          }
        )
      }
      
      message.success(successMessage)
      onRefresh()
    } catch (error: any) {
      console.error('Share action failed:', error)
      message.error(error.message || 'Action failed')
    } finally {
      setActionLoading(null)
    }
  }

  // Show share details
  const showDetails = (share: SharedContainer | ReceivedShare) => {
    setSelectedShare(share)
    setDetailsVisible(true)
  }

  // Render shared container item
  const renderSharedContainer = (share: SharedContainer) => {
    const isExpired = share.expiresAt && dayjs(share.expiresAt).isBefore(dayjs())
    const isLoading = actionLoading === share.id

    const actions = [
      <Tooltip title="View Details">
        <Button
          type="text"
          icon={<EyeOutlined />}
          onClick={() => showDetails(share)}
        />
      </Tooltip>,
      <Popconfirm
        title="Revoke Share"
        description={`Are you sure you want to revoke access for ${share.recipientUsername}?`}
        onConfirm={() => handleShareAction(
          () => onRevokeShare(share.id),
          share.id,
          'Share revoked successfully',
          AuditEventType.SHARE_REVOKED,
          share
        )}
        okText="Revoke"
        cancelText="Cancel"
        okButtonProps={{ danger: true }}
      >
        <Tooltip title="Revoke Share">
          <Button
            type="text"
            danger
            icon={<DeleteOutlined />}
            loading={isLoading}
            disabled={share.status === ShareStatus.REVOKED}
          />
        </Tooltip>
      </Popconfirm>,
    ]

    return (
      <motion.div
        initial={{ opacity: 0, y: 20 }}
        animate={{ opacity: 1, y: 0 }}
        exit={{ opacity: 0, y: -20 }}
        transition={{ duration: 0.3 }}
      >
        <List.Item actions={actions}>
          <List.Item.Meta
            avatar={
              <div className="w-10 h-10 bg-primary-100 dark:bg-primary-900 rounded-lg flex items-center justify-center">
                <ShareAltOutlined className="text-primary-500" />
              </div>
            }
            title={
              <div className="flex items-center space-x-2">
                <span className="font-medium">{share.containerName}</span>
                {getStatusBadge(share.status)}
                {isExpired && <CustomBadge variant="error">Expired</CustomBadge>}
              </div>
            }
            description={
              <div className="space-y-2">
                <div className="flex items-center space-x-4 text-sm text-gray-600 dark:text-gray-400">
                  <span className="flex items-center space-x-1">
                    <UserOutlined />
                    <span>{share.recipientUsername}</span>
                  </span>
                  <span className="flex items-center space-x-1">
                    <CalendarOutlined />
                    <span>{dayjs(share.createdAt).fromNow()}</span>
                  </span>
                  {share.accessCount > 0 && (
                    <span>Accessed {share.accessCount} times</span>
                  )}
                </div>
                <div className="flex items-center space-x-2">
                  {getPermissionTags(share.permissions)}
                </div>
                {share.expiresAt && (
                  <Text type="secondary" className="text-xs">
                    Expires: {dayjs(share.expiresAt).format('MMM D, YYYY HH:mm')}
                  </Text>
                )}
              </div>
            }
          />
        </List.Item>
      </motion.div>
    )
  }

  // Render received share item
  const renderReceivedShare = (share: ReceivedShare) => {
    const isExpired = share.expiresAt && dayjs(share.expiresAt).isBefore(dayjs())
    const isPending = share.status === ShareStatus.PENDING
    const isLoading = actionLoading === share.id

    const actions = [
      <Tooltip title="View Details">
        <Button
          type="text"
          icon={<EyeOutlined />}
          onClick={() => showDetails(share)}
        />
      </Tooltip>,
    ]

    if (isPending) {
      actions.unshift(
        <Tooltip title="Accept Share">
          <Button
            type="text"
            icon={<CheckOutlined />}
            className="text-green-600 hover:text-green-700"
            loading={isLoading}
            onClick={() => handleShareAction(
              () => onAcceptShare(share.id),
              share.id,
              'Share accepted successfully',
              AuditEventType.SHARE_ACCEPTED,
              share
            )}
          />
        </Tooltip>,
        <Tooltip title="Decline Share">
          <Button
            type="text"
            danger
            icon={<CloseOutlined />}
            loading={isLoading}
            onClick={() => handleShareAction(
              () => onDeclineShare(share.id),
              share.id,
              'Share declined',
              AuditEventType.SHARE_DECLINED,
              share
            )}
          />
        </Tooltip>
      )
    }

    return (
      <motion.div
        initial={{ opacity: 0, y: 20 }}
        animate={{ opacity: 1, y: 0 }}
        exit={{ opacity: 0, y: -20 }}
        transition={{ duration: 0.3 }}
      >
        <List.Item actions={actions}>
          <List.Item.Meta
            avatar={
              <div className="w-10 h-10 bg-green-100 dark:bg-green-900 rounded-lg flex items-center justify-center">
                <ShareAltOutlined className="text-green-500" />
              </div>
            }
            title={
              <div className="flex items-center space-x-2">
                <span className="font-medium">{share.containerName}</span>
                {getStatusBadge(share.status)}
                {isExpired && <CustomBadge variant="error">Expired</CustomBadge>}
                {isPending && <Badge dot />}
              </div>
            }
            description={
              <div className="space-y-2">
                <div className="flex items-center space-x-4 text-sm text-gray-600 dark:text-gray-400">
                  <span className="flex items-center space-x-1">
                    <UserOutlined />
                    <span>From {share.senderUsername}</span>
                  </span>
                  <span className="flex items-center space-x-1">
                    <CalendarOutlined />
                    <span>{dayjs(share.createdAt).fromNow()}</span>
                  </span>
                </div>
                <div className="flex items-center space-x-2">
                  {getPermissionTags(share.permissions)}
                </div>
                {share.message && (
                  <Text className="text-sm italic">"{share.message}"</Text>
                )}
                {share.expiresAt && (
                  <Text type="secondary" className="text-xs">
                    Expires: {dayjs(share.expiresAt).format('MMM D, YYYY HH:mm')}
                  </Text>
                )}
              </div>
            }
          />
        </List.Item>
      </motion.div>
    )
  }

  // Status filter options
  const statusFilterOptions = [
    { key: 'all', label: 'All Status' },
    { key: ShareStatus.PENDING, label: 'Pending' },
    { key: ShareStatus.ACCEPTED, label: 'Active' },
    { key: ShareStatus.DECLINED, label: 'Declined' },
    { key: ShareStatus.REVOKED, label: 'Revoked' },
    { key: ShareStatus.EXPIRED, label: 'Expired' },
  ]

  return (
    <div className="space-y-6">
      {/* Header with tabs and controls */}
      <div className="flex items-center justify-between">
        <div className="flex items-center space-x-4">
          <Button.Group>
            <Button
              type={activeTab === 'shared' ? 'primary' : 'default'}
              onClick={() => setActiveTab('shared')}
              icon={<ShareAltOutlined />}
            >
              My Shares ({sharedContainers.length})
            </Button>
            <Button
              type={activeTab === 'received' ? 'primary' : 'default'}
              onClick={() => setActiveTab('received')}
              icon={<UserOutlined />}
            >
              Received ({receivedShares.length})
              {receivedShares.filter(s => s.status === ShareStatus.PENDING).length > 0 && (
                <Badge
                  count={receivedShares.filter(s => s.status === ShareStatus.PENDING).length}
                  size="small"
                  className="ml-2"
                />
              )}
            </Button>
          </Button.Group>
        </div>

        <div className="flex items-center space-x-2">
          <Search
            placeholder="Search containers or users..."
            value={searchText}
            onChange={(e) => setSearchText(e.target.value)}
            style={{ width: 250 }}
            allowClear
          />
          <Dropdown
            menu={{
              items: statusFilterOptions.map(option => ({
                key: option.key,
                label: option.label,
                onClick: () => setStatusFilter(option.key as ShareStatus | 'all'),
              })),
            }}
            trigger={['click']}
          >
            <Button icon={<FilterOutlined />}>
              {statusFilter === 'all' ? 'All Status' : statusFilter}
            </Button>
          </Dropdown>
          <Button
            icon={<ReloadOutlined />}
            onClick={onRefresh}
            loading={loading}
          />
        </div>
      </div>

      {/* Content */}
      <Card className="glass-card">
        <Spin spinning={loading}>
          <AnimatePresence mode="wait">
            {activeTab === 'shared' ? (
              <motion.div
                key="shared"
                initial={{ opacity: 0, x: -20 }}
                animate={{ opacity: 1, x: 0 }}
                exit={{ opacity: 0, x: 20 }}
                transition={{ duration: 0.3 }}
              >
                {filteredSharedContainers.length > 0 ? (
                  <List
                    dataSource={filteredSharedContainers}
                    renderItem={renderSharedContainer}
                    className="space-y-2"
                  />
                ) : (
                  <Empty
                    image={Empty.PRESENTED_IMAGE_SIMPLE}
                    description={
                      searchText || statusFilter !== 'all'
                        ? 'No shares match your filters'
                        : 'No containers shared yet'
                    }
                  />
                )}
              </motion.div>
            ) : (
              <motion.div
                key="received"
                initial={{ opacity: 0, x: 20 }}
                animate={{ opacity: 1, x: 0 }}
                exit={{ opacity: 0, x: -20 }}
                transition={{ duration: 0.3 }}
              >
                {filteredReceivedShares.length > 0 ? (
                  <List
                    dataSource={filteredReceivedShares}
                    renderItem={renderReceivedShare}
                    className="space-y-2"
                  />
                ) : (
                  <Empty
                    image={Empty.PRESENTED_IMAGE_SIMPLE}
                    description={
                      searchText || statusFilter !== 'all'
                        ? 'No shares match your filters'
                        : 'No shares received yet'
                    }
                  />
                )}
              </motion.div>
            )}
          </AnimatePresence>
        </Spin>
      </Card>

      {/* Share Details Modal */}
      <Modal
        title="Share Details"
        open={detailsVisible}
        onCancel={() => setDetailsVisible(false)}
        footer={null}
        width={600}
      >
        {selectedShare && (
          <Descriptions column={1} bordered>
            <Descriptions.Item label="Container">
              {selectedShare.containerName}
            </Descriptions.Item>
            <Descriptions.Item label={activeTab === 'shared' ? 'Shared with' : 'Shared by'}>
              {'recipientUsername' in selectedShare
                ? selectedShare.recipientUsername
                : selectedShare.senderUsername}
            </Descriptions.Item>
            <Descriptions.Item label="Status">
              {getStatusBadge(selectedShare.status)}
            </Descriptions.Item>
            <Descriptions.Item label="Permissions">
              <Space>{getPermissionTags(selectedShare.permissions)}</Space>
            </Descriptions.Item>
            <Descriptions.Item label="Created">
              {dayjs(selectedShare.createdAt).format('MMM D, YYYY HH:mm')}
            </Descriptions.Item>
            {selectedShare.expiresAt && (
              <Descriptions.Item label="Expires">
                {dayjs(selectedShare.expiresAt).format('MMM D, YYYY HH:mm')}
              </Descriptions.Item>
            )}
            {'accessCount' in selectedShare && (
              <Descriptions.Item label="Access Count">
                {selectedShare.accessCount}
                {selectedShare.lastAccessed && (
                  <Text type="secondary" className="ml-2">
                    (Last: {dayjs(selectedShare.lastAccessed).fromNow()})
                  </Text>
                )}
              </Descriptions.Item>
            )}
            {'message' in selectedShare && selectedShare.message && (
              <Descriptions.Item label="Message">
                {selectedShare.message}
              </Descriptions.Item>
            )}
          </Descriptions>
        )}
      </Modal>
    </div>
  )
}

export default SharedContainersList