"""The full body skeleton and the hands sent to the tracker page: all 33 body points, and each hand given to the
right face (by the body's wrist when there's a body, else by distance)."""
import numpy as np

import face_id as F


def body(nose, wrists, vis=0.9):
    """A 33-point pose with the nose and both wrists (15, 16) placed, everything else unseen."""
    p = np.zeros((33, 3), np.float32)
    p[0] = [*nose, vis]
    p[15] = [*wrists[0], vis]
    p[16] = [*wrists[1], vis]
    return p


def hand(wrist):
    """A hand: 21 points fanning up from the wrist."""
    return np.array([[wrist[0] + i, wrist[1] - 2 * i] for i in range(21)], np.float32)


def test_pose_all_sends_every_point_rounded():
    p = body((100.04, 50.06), ((80, 200), (120, 200)))
    out = F.pose_all(p)
    assert len(out) == 33 and out[0] == [100.0, 50.1, 0.9] and out[1] == [0.0, 0.0, 0.0]


def test_each_hand_goes_to_the_face_whose_wrist_it_is():
    boxes = [[50, 0, 100, 100], [450, 0, 100, 100]]
    poses = [body((100, 50), ((60, 300), (140, 300))), body((500, 50), ((460, 300), (540, 300)))]
    bodies = [0, 1]
    # a hand at face 1's left wrist, though nearer face 0's box centre than face 1's would be without the body
    hands = [hand((140, 305)), hand((462, 298))]
    out = F.hands_for_faces(hands, boxes, poses, bodies)
    assert len(out[0]) == 1 and out[0][0][0] == [140.0, 305.0]
    assert len(out[1]) == 1 and out[1][0][0] == [462.0, 298.0]
    assert len(out[0][0]) == 21


def test_without_a_body_the_nearest_face_within_reach_gets_the_hand():
    boxes = [[0, 0, 100, 100], [600, 0, 100, 100]]
    out = F.hands_for_faces([hand((60, 250)), hand((2000, 2000))], boxes, [], [None, None])
    assert len(out[0]) == 1 and out[1] == []           # the far-away hand belongs to nobody


def test_unseen_wrists_dont_claim_a_hand():
    boxes = [[0, 0, 100, 100], [300, 0, 100, 100]]
    poses = [body((50, 50), ((330, 220), (330, 220)), vis=0.1)]   # face 0's "wrists" are unseen, under face 1
    out = F.hands_for_faces([hand((330, 220))], boxes, poses, [0, None])
    assert out[0] == [] and len(out[1]) == 1
