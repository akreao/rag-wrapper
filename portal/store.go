package main

import (
	"context"
	"database/sql"
	"errors"
	"strings"
	"sync"
	"time"
)

// blockedState is the login.state value rAthena's @block command sets: the
// login server refuses the account until state is back to 0.
const blockedState = 5

// Account is a row of rAthena's login table. Server accounts (sex 'S', such
// as the inter-server login) are never loaded.
type Account struct {
	ID         int
	UserID     string
	Sex        string
	Email      string
	GroupID    int
	State      int64
	UnbanTime  int64
	LoginCount int
	LastLogin  string
	LastIP     string
	Birthdate  string // YYYY-MM-DD, or "" when unset
	passHash   string
}

// Blocked reports whether the login server would refuse this account.
func (a *Account) Blocked() bool { return a.State != 0 }

// Banned reports whether a temporary ban is still running.
func (a *Account) Banned() bool { return a.UnbanTime > time.Now().Unix() }

// BannedUntil is the end of the temporary ban.
func (a *Account) BannedUntil() time.Time { return time.Unix(a.UnbanTime, 0) }

// Character is a row of rAthena's char table.
type Character struct {
	ID        int
	AccountID int
	Slot      int
	Name      string
	Class     int
	BaseLevel int
	JobLevel  int
	Zeny      int64
	LastMap   string
	LastX     int
	LastY     int
	SaveMap   string
	SaveX     int
	SaveY     int
	Online    bool
	Deleting  bool
}

// InTown reports whether the character is in the fallback town and saved there.
func (c Character) InTown() bool {
	return c.AtSavePoint() && c.SaveMap == townMap && c.SaveX == townX && c.SaveY == townY
}

// AtSavePoint reports whether the character would log in at its save point.
func (c Character) AtSavePoint() bool {
	return c.LastMap == c.SaveMap && c.LastX == c.SaveX && c.LastY == c.SaveY
}

func (c Character) JobName() string { return jobName(c.Class) }

var errUserIDTaken = errors.New("that username is already taken")

type store struct {
	db *sql.DB
	// The login table has no unique key on userid, so registrations are
	// serialised here to keep two people from claiming the same name.
	registerMu sync.Mutex
}

const accountColumns = "`account_id`, `userid`, `user_pass`, `sex`, `email`, `group_id`, `state`, `unban_time`, `logincount`, " +
	"COALESCE(CAST(`lastlogin` AS CHAR), ''), `last_ip`, COALESCE(CAST(`birthdate` AS CHAR), '')"

func scanAccount(row interface{ Scan(...any) error }) (*Account, error) {
	a := &Account{}
	err := row.Scan(&a.ID, &a.UserID, &a.passHash, &a.Sex, &a.Email, &a.GroupID, &a.State, &a.UnbanTime,
		&a.LoginCount, &a.LastLogin, &a.LastIP, &a.Birthdate)
	if err != nil {
		return nil, err
	}
	if a.Birthdate == "0000-00-00" {
		a.Birthdate = ""
	}
	return a, nil
}

// accountByUserID returns nil, nil when there's no such account.
func (s *store) accountByUserID(ctx context.Context, userID string) (*Account, error) {
	a, err := scanAccount(s.db.QueryRowContext(ctx,
		"SELECT "+accountColumns+" FROM `login` WHERE `userid` = ? AND `sex` <> 'S'", userID))
	if errors.Is(err, sql.ErrNoRows) {
		return nil, nil
	}
	return a, err
}

// accountByID returns nil, nil when there's no such account.
func (s *store) accountByID(ctx context.Context, id int) (*Account, error) {
	a, err := scanAccount(s.db.QueryRowContext(ctx,
		"SELECT "+accountColumns+" FROM `login` WHERE `account_id` = ? AND `sex` <> 'S'", id))
	if errors.Is(err, sql.ErrNoRows) {
		return nil, nil
	}
	return a, err
}

func (s *store) createAccount(ctx context.Context, userID, passHash, sex, email, birthdate string) (int, error) {
	s.registerMu.Lock()
	defer s.registerMu.Unlock()

	// userid uses a case-insensitive collation, as the login server's
	// lookups do, so "Bob" and "bob" count as the same name.
	var exists int
	err := s.db.QueryRowContext(ctx, "SELECT COUNT(*) FROM `login` WHERE `userid` = ?", userID).Scan(&exists)
	if err != nil {
		return 0, err
	}
	if exists > 0 {
		return 0, errUserIDTaken
	}
	res, err := s.db.ExecContext(ctx,
		"INSERT INTO `login` (`userid`, `user_pass`, `sex`, `email`, `birthdate`) VALUES (?, ?, ?, ?, ?)",
		userID, passHash, sex, email, nullIfEmpty(birthdate))
	if err != nil {
		return 0, err
	}
	id, err := res.LastInsertId()
	return int(id), err
}

func (s *store) setPassword(ctx context.Context, id int, passHash string) error {
	_, err := s.db.ExecContext(ctx, "UPDATE `login` SET `user_pass` = ? WHERE `account_id` = ? AND `sex` <> 'S'", passHash, id)
	return err
}

func (s *store) setDetails(ctx context.Context, id int, email, birthdate string) error {
	_, err := s.db.ExecContext(ctx, "UPDATE `login` SET `email` = ?, `birthdate` = ? WHERE `account_id` = ? AND `sex` <> 'S'",
		email, nullIfEmpty(birthdate), id)
	return err
}

const characterColumns = "`char_id`, `account_id`, `char_num`, `name`, `class`, `base_level`, `job_level`, `zeny`, " +
	"`last_map`, `last_x`, `last_y`, `save_map`, `save_x`, `save_y`, `online`, `delete_date`"

func scanCharacter(row interface{ Scan(...any) error }) (Character, error) {
	var c Character
	var online, deleteDate int64
	err := row.Scan(&c.ID, &c.AccountID, &c.Slot, &c.Name, &c.Class, &c.BaseLevel, &c.JobLevel, &c.Zeny,
		&c.LastMap, &c.LastX, &c.LastY, &c.SaveMap, &c.SaveX, &c.SaveY, &online, &deleteDate)
	c.Online = online != 0
	c.Deleting = deleteDate != 0
	return c, err
}

func (s *store) characters(ctx context.Context, accountID int) ([]Character, error) {
	rows, err := s.db.QueryContext(ctx,
		"SELECT "+characterColumns+" FROM `char` WHERE `account_id` = ? ORDER BY `char_num`", accountID)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var chars []Character
	for rows.Next() {
		c, err := scanCharacter(rows)
		if err != nil {
			return nil, err
		}
		chars = append(chars, c)
	}
	return chars, rows.Err()
}

// characterByID returns nil, nil when there's no such character.
func (s *store) characterByID(ctx context.Context, id int) (*Character, error) {
	c, err := scanCharacter(s.db.QueryRowContext(ctx, "SELECT "+characterColumns+" FROM `char` WHERE `char_id` = ?", id))
	if errors.Is(err, sql.ErrNoRows) {
		return nil, nil
	}
	if err != nil {
		return nil, err
	}
	return &c, nil
}

var errCharacterOnline = errors.New("character is online")

// The town a character is sent to when its save point is the problem:
// rAthena's own fallback (default_map in char_athena.conf).
const (
	townMap = "prontera"
	townX   = 156
	townY   = 191
)

// moveToSavePoint puts an offline character back at its save point.
func (s *store) moveToSavePoint(ctx context.Context, id int) error {
	return s.moveOffline(ctx, id, "`last_map` = `save_map`, `last_x` = `save_x`, `last_y` = `save_y`")
}

// moveToTown puts an offline character in Prontera and saves it there,
// for a character whose save point is itself where it's stuck.
func (s *store) moveToTown(ctx context.Context, id int) error {
	return s.moveOffline(ctx, id, "`last_map` = ?, `last_x` = ?, `last_y` = ?, `save_map` = ?, `save_x` = ?, `save_y` = ?",
		townMap, townX, townY, townMap, townX, townY)
}

// moveOffline changes where a character logs in. The char server reads the
// position from the database when the character is next selected, so this
// is only safe while it is offline; the online check is part of the
// update, so a player who logs in at the same moment wins.
func (s *store) moveOffline(ctx context.Context, id int, set string, args ...any) error {
	res, err := s.db.ExecContext(ctx,
		"UPDATE `char` SET "+set+", `last_instanceid` = 0 WHERE `char_id` = ? AND `online` = 0", append(args, id)...)
	if err != nil {
		return err
	}
	if n, err := res.RowsAffected(); err != nil || n > 0 {
		return err
	}
	// Nothing changed: either it's online, or it was already there.
	var online int
	if err := s.db.QueryRowContext(ctx, "SELECT `online` FROM `char` WHERE `char_id` = ?", id).Scan(&online); err != nil {
		return err
	}
	if online != 0 {
		return errCharacterOnline
	}
	return nil
}

// searchAccounts lists player accounts whose name or email, or the name of
// one of whose characters, contains q, newest first.
func (s *store) searchAccounts(ctx context.Context, q string, limit, offset int) ([]*Account, error) {
	like := "%" + escapeLike(q) + "%"
	rows, err := s.db.QueryContext(ctx,
		"SELECT "+accountColumns+" FROM `login` WHERE `sex` <> 'S' AND (`userid` LIKE ? OR `email` LIKE ? "+
			"OR `account_id` IN (SELECT `account_id` FROM `char` WHERE `name` LIKE ?)) "+
			"ORDER BY `account_id` DESC LIMIT ? OFFSET ?", like, like, like, limit, offset)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var accounts []*Account
	for rows.Next() {
		a, err := scanAccount(rows)
		if err != nil {
			return nil, err
		}
		accounts = append(accounts, a)
	}
	return accounts, rows.Err()
}

func (s *store) setGroup(ctx context.Context, id, group int) error {
	_, err := s.db.ExecContext(ctx, "UPDATE `login` SET `group_id` = ? WHERE `account_id` = ? AND `sex` <> 'S'", group, id)
	return err
}

// ban refuses logins until the given time, like @ban.
func (s *store) ban(ctx context.Context, id int, until time.Time) error {
	_, err := s.db.ExecContext(ctx, "UPDATE `login` SET `unban_time` = ? WHERE `account_id` = ? AND `sex` <> 'S'", until.Unix(), id)
	return err
}

// block refuses logins until unblocked, like @block.
func (s *store) block(ctx context.Context, id int) error {
	_, err := s.db.ExecContext(ctx, "UPDATE `login` SET `state` = ? WHERE `account_id` = ? AND `sex` <> 'S'", blockedState, id)
	return err
}

// unban lifts both a temporary ban and a block.
func (s *store) unban(ctx context.Context, id int) error {
	_, err := s.db.ExecContext(ctx, "UPDATE `login` SET `state` = 0, `unban_time` = 0 WHERE `account_id` = ? AND `sex` <> 'S'", id)
	return err
}

func (s *store) ping(ctx context.Context) error { return s.db.PingContext(ctx) }

func nullIfEmpty(s string) any {
	if s == "" {
		return nil
	}
	return s
}

func escapeLike(s string) string {
	return strings.NewReplacer(`\`, `\\`, `%`, `\%`, `_`, `\_`).Replace(s)
}
